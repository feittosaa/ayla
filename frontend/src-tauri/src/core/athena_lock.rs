//! Pastas protegidas da Athena: o conteúdo fica cifrado no disco (e no Git), e só a Athena o
//! mostra, depois da senha.
//!
//! Formato, dentro da pasta protegida:
//! - `.athena-lock.json`: parâmetros do Argon2id, sal e a chave dos dados cifrada com a chave
//!   derivada da senha (trocar a senha não exige recifrar os arquivos);
//! - `<id>.athena`, um por arquivo ou subpasta: `AYLALCK1`, tamanho do cabeçalho (u32), cabeçalho
//!   cifrado (caminho, tipo, data e tamanho, em JSON) e o conteúdo cifrado. Nomes e pastas também
//!   ficam cifrados; no disco só aparecem identificadores aleatórios.
//!
//! Cifra: XChaCha20-Poly1305 com nonce aleatório de 24 bytes; o identificador do arquivo entra
//! como dado associado, para um cabeçalho ou conteúdo não ser trocado de arquivo.
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub const LOCK_FILE: &str = ".athena-lock.json";
const EXTENSION: &str = "athena";
const MAGIC: &[u8; 8] = b"AYLALCK1";
/// Limite por arquivo dentro de uma pasta protegida (o arquivo é cifrado inteiro, na memória).
pub const MAX_FILE: u64 = 100 * 1024 * 1024;
pub const MIN_PASSWORD: usize = 8;
/// Argon2id: 64 MiB e 3 passagens (OWASP); nos testes, o mínimo para rodarem rápido.
const MEMORY_KIB: u32 = if cfg!(test) { 1024 } else { 64 * 1024 };
const ITERATIONS: u32 = if cfg!(test) { 1 } else { 3 };

#[derive(Serialize, Deserialize)]
struct LockFile {
    version: u32,
    kdf: String,
    memory: u32,
    iterations: u32,
    parallelism: u32,
    salt: String,
    key: String,
    /// A mesma chave dos dados, cifrada pela chave de recuperação (pastas protegidas antes dela
    /// existir não têm; dá para criar com a senha).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recovery: Option<Wrapped>,
    /// A chave dos dados cifrada para a chave de recuperação global.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    global: Option<GlobalWrap>,
}

/// Chave dos dados cifrada por uma chave derivada (Argon2id) de um segredo.
#[derive(Clone, Serialize, Deserialize)]
struct Wrapped {
    memory: u32,
    iterations: u32,
    parallelism: u32,
    salt: String,
    key: String,
}

/// Dado associado da cópia da chave cifrada pela chave de recuperação.
const RECOVERY_AAD: &[u8; 8] = b"AYLARCV1";
/// Base32 de Crockford: sem I, L, O e U, que se confundem ao anotar.
const RECOVERY_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Chave dos dados de uma pasta, apagada da memória quando sai de uso.
#[derive(Clone)]
pub struct Key(Zeroizing<[u8; 32]>);

/// Arquivo ou subpasta dentro da pasta protegida (caminho relativo a ela).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub kind: String,
    pub modified: i64,
    pub bytes: u64,
    #[serde(skip)]
    pub id: String,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn random<const N: usize>() -> Result<[u8; N], String> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|_| "Não foi possível gerar dados aleatórios")?;
    Ok(bytes)
}

fn derive(password: &str, salt: &[u8], memory: u32, iterations: u32, parallelism: u32) -> Result<Zeroizing<[u8; 32]>, String> {
    let params = Params::new(memory, iterations, parallelism, Some(32)).map_err(|_| "Proteção da pasta inválida")?;
    let mut out = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, out.as_mut())
        .map_err(|_| "Não foi possível derivar a chave da senha")?;
    Ok(out)
}

fn seal(key: &[u8; 32], aad: &[u8], plain: &[u8]) -> Result<Vec<u8>, String> {
    let nonce = random::<24>()?;
    let cipher = XChaCha20Poly1305::new(key.into());
    let sealed = cipher.encrypt(XNonce::from_slice(&nonce), Payload { msg: plain, aad }).map_err(|_| "Falha ao cifrar")?;
    Ok([nonce.as_slice(), &sealed].concat())
}

fn open(key: &[u8; 32], aad: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 24 {
        return None;
    }
    let cipher = XChaCha20Poly1305::new(key.into());
    cipher.decrypt(XNonce::from_slice(&data[..24]), Payload { msg: &data[24..], aad }).ok()
}

/// Caminho relativo seguro dentro da pasta protegida (sem `..`, absolutos ou partes ocultas).
fn clean(relative: &str) -> Result<String, String> {
    let path = Path::new(relative);
    let ok = !relative.is_empty() && relative.len() <= 1024
        && path.components().all(|c| matches!(c, Component::Normal(part) if !part.to_string_lossy().starts_with('.')));
    if ok { Ok(path.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")) } else { Err("Caminho inválido".into()) }
}

/// A pasta (caminho absoluto) está protegida?
pub fn is_protected(dir: &Path) -> bool {
    dir.join(LOCK_FILE).is_file()
}

/// Pasta protegida que contém o caminho (relativo à raiz), se houver. O próprio caminho conta
/// quando é a pasta protegida.
pub fn protected_folder_of(root: &Path, relative: &str) -> Option<String> {
    let mut prefix = PathBuf::new();
    for part in Path::new(relative).components() {
        let Component::Normal(part) = part else { return None };
        prefix.push(part);
        if is_protected(&root.join(&prefix)) {
            return Some(prefix.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"));
        }
    }
    None
}

/// Pastas protegidas da raiz (sem entrar nelas nem em pastas ocultas).
pub fn protected_folders(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(root.join(&dir)) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let relative = dir.join(&name);
            if is_protected(&root.join(&relative)) {
                out.push(relative.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"));
            } else {
                pending.push(relative);
            }
        }
    }
    out.sort();
    out
}

fn file_of(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.{EXTENSION}"))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_file_name(format!(".{}.tmp", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
    let result = (|| -> std::io::Result<()> {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
        return Err("Não foi possível gravar na pasta protegida".into());
    }
    Ok(())
}

fn encode(key: &Key, id: &str, entry: &Entry, body: &[u8]) -> Result<Vec<u8>, String> {
    let header = serde_json::to_vec(entry).map_err(|_| "Falha ao cifrar")?;
    let header = seal(&key.0, &[MAGIC.as_slice(), id.as_bytes()].concat(), &header)?;
    let body = seal(&key.0, id.as_bytes(), body)?;
    let mut out = Vec::with_capacity(12 + header.len() + body.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(header.len() as u32).to_be_bytes());
    out.extend_from_slice(&header);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Lê só o cabeçalho (o caminho e a data), sem decifrar o conteúdo.
fn read_header(key: &Key, path: &Path) -> Option<(Entry, u64)> {
    let id = path.file_stem()?.to_string_lossy().to_string();
    let mut file = fs::File::open(path).ok()?;
    let mut start = [0u8; 12];
    file.read_exact(&mut start).ok()?;
    if &start[..8] != MAGIC {
        return None;
    }
    let length = u32::from_be_bytes(start[8..12].try_into().ok()?) as u64;
    if length > 64 * 1024 {
        return None;
    }
    let mut header = vec![0u8; length as usize];
    file.read_exact(&mut header).ok()?;
    let plain = open(&key.0, &[MAGIC.as_slice(), id.as_bytes()].concat(), &header)?;
    let mut entry: Entry = serde_json::from_slice(&plain).ok()?;
    entry.id = id;
    Some((entry, 12 + length))
}

/// Arquivos e subpastas da pasta protegida, com a chave dela.
pub fn entries(dir: &Path, key: &Key) -> Vec<Entry> {
    let Ok(list) = fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<Entry> = list.flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == EXTENSION))
        .filter_map(|p| read_header(key, &p).map(|(entry, _)| entry))
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn find(dir: &Path, key: &Key, path: &str) -> Option<Entry> {
    entries(dir, key).into_iter().find(|e| e.path == path)
}

/// Conteúdo de um arquivo da pasta protegida.
pub fn read(dir: &Path, key: &Key, path: &str) -> Result<(Vec<u8>, Entry), String> {
    let path = clean(path)?;
    let entry = find(dir, key, &path).filter(|e| e.kind == "file").ok_or("Arquivo não encontrado na pasta protegida")?;
    let file = file_of(dir, &entry.id);
    let (_, offset) = read_header(key, &file).ok_or("Arquivo da pasta protegida danificado")?;
    let bytes = fs::read(&file).map_err(|_| "Não foi possível ler a pasta protegida")?;
    let body = open(&key.0, entry.id.as_bytes(), &bytes[offset as usize..]).ok_or("Arquivo da pasta protegida danificado")?;
    Ok((body, entry))
}

/// Grava (cria ou substitui) um arquivo; recusa se mudou desde `expected`. Cria as subpastas.
pub fn write(dir: &Path, key: &Key, path: &str, body: &[u8], expected: Option<i64>) -> Result<Entry, String> {
    let path = clean(path)?;
    if body.len() as u64 > MAX_FILE {
        return Err("Arquivos de pastas protegidas vão até 100 MB".into());
    }
    let all = entries(dir, key);
    if all.iter().any(|e| e.path == path && e.kind == "folder") {
        return Err("Já existe uma pasta com esse nome".into());
    }
    let existing = all.iter().find(|e| e.path == path);
    if let (Some(existing), Some(expected)) = (existing, expected) {
        if existing.modified > expected {
            return Err("Esta nota mudou desde que foi aberta. Ela foi recarregada; refaça a última alteração.".into());
        }
    }
    // A data só avança: duas gravações no mesmo segundo não parecem conflito na seguinte.
    let modified = existing.map_or(now(), |e| now().max(e.modified));
    let id = match existing { Some(e) => e.id.clone(), None => hex(&random::<16>()?) };
    let entry = Entry { path: path.clone(), kind: "file".into(), modified, bytes: body.len() as u64, id: id.clone() };
    write_atomic(&file_of(dir, &id), &encode(key, &id, &entry, body)?)?;
    ensure_parents(dir, key, &path, &all)?;
    Ok(entry)
}

/// Cria as subpastas acima do caminho que ainda não existem.
fn ensure_parents(dir: &Path, key: &Key, path: &str, all: &[Entry]) -> Result<(), String> {
    for folder in Path::new(path).ancestors().skip(1).filter(|p| !p.as_os_str().is_empty()) {
        let folder = folder.to_string_lossy();
        if !all.iter().any(|e| e.path == folder) {
            add_folder(dir, key, &folder)?;
        }
    }
    Ok(())
}

/// Subpasta (pode ficar vazia, como no disco).
pub fn add_folder(dir: &Path, key: &Key, path: &str) -> Result<(), String> {
    let path = clean(path)?;
    if find(dir, key, &path).is_some() {
        return Ok(());
    }
    let id = hex(&random::<16>()?);
    let entry = Entry { path, kind: "folder".into(), modified: now(), bytes: 0, id: id.clone() };
    write_atomic(&file_of(dir, &id), &encode(key, &id, &entry, &[])?)
}

fn rewrite_header(dir: &Path, key: &Key, entry: &Entry) -> Result<(), String> {
    let file = file_of(dir, &entry.id);
    let (_, offset) = read_header(key, &file).ok_or("Arquivo da pasta protegida danificado")?;
    let bytes = fs::read(&file).map_err(|_| "Não foi possível ler a pasta protegida")?;
    let header = serde_json::to_vec(entry).map_err(|_| "Falha ao cifrar")?;
    let header = seal(&key.0, &[MAGIC.as_slice(), entry.id.as_bytes()].concat(), &header)?;
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(header.len() as u32).to_be_bytes());
    out.extend_from_slice(&header);
    out.extend_from_slice(&bytes[offset as usize..]);
    write_atomic(&file, &out)
}

/// Renomeia ou move um arquivo ou subpasta dentro da pasta protegida (o conteúdo não é recifrado).
pub fn rename(dir: &Path, key: &Key, from: &str, to: &str) -> Result<(), String> {
    let (from, to) = (clean(from)?, clean(to)?);
    let all = entries(dir, key);
    if all.iter().any(|e| e.path == to) {
        return Err("Já existe um arquivo com esse nome".into());
    }
    let prefix = format!("{from}/");
    let moving: Vec<&Entry> = all.iter().filter(|e| e.path == from || e.path.starts_with(&prefix)).collect();
    if moving.is_empty() {
        return Err("Arquivo não encontrado na pasta protegida".into());
    }
    for entry in moving {
        let mut changed = entry.clone();
        changed.path = format!("{to}{}", &entry.path[from.len()..]);
        rewrite_header(dir, key, &changed)?;
    }
    ensure_parents(dir, key, &to, &all)?;
    Ok(())
}

/// Leva um arquivo ou subpasta (com o que tiver dentro) para a lixeira da raiz, ainda cifrado.
pub fn trash(dir: &Path, key: &Key, path: &str, trash: &Path) -> Result<(), String> {
    let path = clean(path)?;
    let prefix = format!("{path}/");
    let gone: Vec<Entry> = entries(dir, key).into_iter().filter(|e| e.path == path || e.path.starts_with(&prefix)).collect();
    if gone.is_empty() {
        return Err("Arquivo não encontrado na pasta protegida".into());
    }
    fs::create_dir_all(trash).map_err(|_| "Não foi possível criar a lixeira")?;
    for entry in gone {
        fs::rename(file_of(dir, &entry.id), trash.join(format!("{}.{EXTENSION}", entry.id))).map_err(|_| "Não foi possível mover para a lixeira")?;
    }
    Ok(())
}

/// Apaga de vez (o conteúdo já existe em outro lugar, por exemplo ao mover para fora da proteção).
pub fn remove(dir: &Path, key: &Key, path: &str) -> Result<(), String> {
    let path = clean(path)?;
    let prefix = format!("{path}/");
    for entry in entries(dir, key).into_iter().filter(|e| e.path == path || e.path.starts_with(&prefix)) {
        fs::remove_file(file_of(dir, &entry.id)).map_err(|_| "Não foi possível apagar da pasta protegida")?;
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Onde fica a proteção: numa pasta (`.athena-lock.json`, arquivos cifrados ao lado) ou no próprio
/// arquivo protegido sozinho (`Nome.md.athena`: proteção no cabeçalho e o conteúdo cifrado).
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Folder(PathBuf),
    /// Caminho do `.athena` no disco.
    File(PathBuf),
}

impl Target {
    pub fn path(&self) -> &Path {
        match self { Target::Folder(path) | Target::File(path) => path }
    }
}

/// Lê a proteção (e, num arquivo, o conteúdo cifrado que vem depois dela).
fn read_target(target: &Target) -> Result<(LockFile, Vec<u8>), String> {
    let (header, body) = match target {
        Target::Folder(dir) => (fs::read(dir.join(LOCK_FILE)).map_err(|_| "Esta pasta não está protegida")?, Vec::new()),
        Target::File(path) => {
            let bytes = fs::read(path).map_err(|_| "Arquivo protegido não encontrado")?;
            if bytes.len() < 12 || &bytes[..8] != FILE_MAGIC {
                return Err("Arquivo protegido danificado".into());
            }
            let length = u32::from_be_bytes(bytes[8..12].try_into().map_err(|_| "Arquivo protegido danificado")?) as usize;
            if bytes.len() < 12 + length || length > 64 * 1024 {
                return Err("Arquivo protegido danificado".into());
            }
            (bytes[12..12 + length].to_vec(), bytes[12 + length..].to_vec())
        }
    };
    let lock: LockFile = serde_json::from_slice(&header).map_err(|_| "Proteção danificada")?;
    if lock.version != 1 || lock.kdf != "argon2id" {
        return Err("Proteção em formato desconhecido".into());
    }
    Ok((lock, body))
}

fn write_target(target: &Target, lock: &LockFile, body: &[u8]) -> Result<(), String> {
    let header = serde_json::to_vec_pretty(lock).map_err(|_| "Falha ao salvar a proteção")?;
    match target {
        Target::Folder(dir) => write_atomic(&dir.join(LOCK_FILE), &header),
        Target::File(path) => {
            let mut out = Vec::with_capacity(12 + header.len() + body.len());
            out.extend_from_slice(FILE_MAGIC);
            out.extend_from_slice(&(header.len() as u32).to_be_bytes());
            out.extend_from_slice(&header);
            out.extend_from_slice(body);
            write_atomic(path, &out)
        }
    }
}

fn wrap(secret: &str, key: &Key, aad: &[u8]) -> Result<Wrapped, String> {
    let salt = random::<16>()?;
    let kek = derive(secret, &salt, MEMORY_KIB, ITERATIONS, 1)?;
    Ok(Wrapped { memory: MEMORY_KIB, iterations: ITERATIONS, parallelism: 1, salt: B64.encode(salt), key: B64.encode(seal(&kek, aad, key.0.as_slice())?) })
}

fn unwrap(wrapped: &Wrapped, secret: &str, aad: &[u8], wrong: &str) -> Result<Key, String> {
    if wrapped.memory > 1024 * 1024 || wrapped.iterations > 20 || wrapped.parallelism > 8 {
        return Err("Proteção em formato desconhecido".into());
    }
    let salt = B64.decode(&wrapped.salt).map_err(|_| "Proteção danificada")?;
    let sealed = B64.decode(&wrapped.key).map_err(|_| "Proteção danificada")?;
    let kek = derive(secret, &salt, wrapped.memory, wrapped.iterations, wrapped.parallelism)?;
    let plain = Zeroizing::new(open(&kek, aad, &sealed).ok_or(wrong)?);
    let key: [u8; 32] = plain.as_slice().try_into().map_err(|_| "Proteção danificada")?;
    Ok(Key(Zeroizing::new(key)))
}

fn main_key(lock: &LockFile) -> Wrapped {
    Wrapped { memory: lock.memory, iterations: lock.iterations, parallelism: lock.parallelism, salt: lock.salt.clone(), key: lock.key.clone() }
}

fn check_password(password: &str) -> Result<(), String> {
    if password.chars().count() < MIN_PASSWORD {
        return Err(format!("Use uma senha com pelo menos {MIN_PASSWORD} caracteres"));
    }
    Ok(())
}

/// Proteção com a senha (e a chave global, se houver) para a chave dos dados.
fn new_lock(key: &Key, password: &str, recovery: Option<Wrapped>, global: Option<GlobalWrap>) -> Result<LockFile, String> {
    check_password(password)?;
    let main = wrap(password, key, MAGIC)?;
    Ok(LockFile { version: 1, kdf: "argon2id".into(), memory: main.memory, iterations: main.iterations, parallelism: main.parallelism, salt: main.salt, key: main.key, recovery, global })
}

/// Confere a senha e devolve a chave dos dados.
pub fn unlock(target: &Target, password: &str) -> Result<Key, String> {
    let (lock, _) = read_target(target)?;
    unwrap(&main_key(&lock), password, MAGIC, "Senha incorreta")
}

/// Troca a senha (só a chave dos dados é recifrada); a recuperação continua valendo.
pub fn change_password(target: &Target, current: &str, new: &str) -> Result<(), String> {
    let (lock, body) = read_target(target)?;
    let key = unwrap(&main_key(&lock), current, MAGIC, "Senha incorreta")?;
    write_target(target, &new_lock(&key, new, lock.recovery, lock.global)?, &body)
}

// ---------- Chave de recuperação global ----------
// Um código anotado (`ATHENA-XXXX-…`, 160 bits) gera, sempre igual, um par de chaves X25519. O
// computador guarda só a parte pública; cada item protegido guarda a chave dos dados dele
// cifrada para essa chave pública. Só o código (a parte privada) abre essas cópias.

/// Parâmetros fixos (o código já tem 160 bits; o Argon2 só dificulta tentativas em massa).
const GLOBAL_SALT: &[u8] = b"ayla-athena-recuperacao-global-v1";
const GLOBAL_MEMORY_KIB: u32 = 64 * 1024;
const GLOBAL_ITERATIONS: u32 = 3;
const GLOBAL_AAD: &[u8; 8] = b"AYLARCV2";

/// Cópia da chave dos dados para a chave pública global (com uma chave efêmera própria).
#[derive(Clone, Serialize, Deserialize)]
pub struct GlobalWrap {
    public: String,
    ephemeral: String,
    key: String,
}

fn global_secret(code: &str) -> Result<x25519_dalek::StaticSecret, String> {
    let code = normalize_code(code).ok_or("Chave de recuperação incorreta")?;
    let seed = derive(&code, GLOBAL_SALT, GLOBAL_MEMORY_KIB, GLOBAL_ITERATIONS, 1)?;
    Ok(x25519_dalek::StaticSecret::from(*seed))
}

/// Chave pública que corresponde ao código (para ativar a recuperação em outro computador).
pub fn global_public(code: &str) -> Result<[u8; 32], String> {
    Ok(x25519_dalek::PublicKey::from(&global_secret(code)?).to_bytes())
}

/// Chave global nova: o código (mostrado uma única vez) e a parte pública, que fica guardada.
pub fn new_global() -> Result<(String, [u8; 32]), String> {
    let code = recovery_code()?;
    let public = global_public(&code)?;
    Ok((code, public))
}

fn global_kek(shared: &[u8; 32], ephemeral: &[u8; 32], public: &[u8; 32]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::new().chain_update(GLOBAL_AAD).chain_update(shared).chain_update(ephemeral).chain_update(public).finalize().into()
}

fn wrap_global(public: &[u8; 32], key: &Key) -> Result<GlobalWrap, String> {
    let ephemeral = x25519_dalek::StaticSecret::from(random::<32>()?);
    let ephemeral_public = x25519_dalek::PublicKey::from(&ephemeral).to_bytes();
    let shared = Zeroizing::new(ephemeral.diffie_hellman(&x25519_dalek::PublicKey::from(*public)).to_bytes());
    let kek = Zeroizing::new(global_kek(&shared, &ephemeral_public, public));
    Ok(GlobalWrap { public: B64.encode(public), ephemeral: B64.encode(ephemeral_public), key: B64.encode(seal(&kek, GLOBAL_AAD, key.0.as_slice())?) })
}

fn unwrap_global(wrapped: &GlobalWrap, code: &str) -> Result<Key, String> {
    let secret = global_secret(code)?;
    let public: [u8; 32] = B64.decode(&wrapped.public).ok().and_then(|b| b.try_into().ok()).ok_or("Proteção danificada")?;
    if x25519_dalek::PublicKey::from(&secret).to_bytes() != public {
        return Err("Chave de recuperação incorreta".into());
    }
    let ephemeral: [u8; 32] = B64.decode(&wrapped.ephemeral).ok().and_then(|b| b.try_into().ok()).ok_or("Proteção danificada")?;
    let sealed = B64.decode(&wrapped.key).map_err(|_| "Proteção danificada")?;
    let shared = Zeroizing::new(secret.diffie_hellman(&x25519_dalek::PublicKey::from(ephemeral)).to_bytes());
    let kek = Zeroizing::new(global_kek(&shared, &ephemeral, &public));
    let plain = Zeroizing::new(open(&kek, GLOBAL_AAD, &sealed).ok_or("Chave de recuperação incorreta")?);
    let key: [u8; 32] = plain.as_slice().try_into().map_err(|_| "Proteção danificada")?;
    Ok(Key(Zeroizing::new(key)))
}

/// Garante a cópia para a chave global atual (itens protegidos antes dela, ou com uma chave global
/// antiga, ganham a cópia nova na próxima vez que forem abertos). Devolve se mudou algo.
pub fn ensure_global(target: &Target, key: &Key, public: &[u8; 32]) -> Result<bool, String> {
    let (mut lock, body) = read_target(target)?;
    if lock.global.as_ref().is_some_and(|g| g.public == B64.encode(public)) {
        return Ok(false);
    }
    lock.global = Some(wrap_global(public, key)?);
    write_target(target, &lock, &body)?;
    Ok(true)
}

/// Parte pública da chave global com que o item foi protegido (para conferir um código digitado).
pub fn global_of(target: &Target) -> Option<[u8; 32]> {
    let (lock, _) = read_target(target).ok()?;
    B64.decode(lock.global?.public).ok()?.try_into().ok()
}

/// Chave de recuperação nova: 160 bits aleatórios, em grupos de 4 (`ATHENA-XXXX-…`).
fn recovery_code() -> Result<String, String> {
    let bytes = random::<20>()?;
    let mut bits = 0u64;
    let mut count = 0;
    let mut out = String::new();
    for byte in bytes {
        bits = (bits << 8) | byte as u64;
        count += 8;
        while count >= 5 {
            count -= 5;
            out.push(RECOVERY_ALPHABET[((bits >> count) & 31) as usize] as char);
        }
    }
    let groups: Vec<String> = out.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).into_owned()).collect();
    Ok(format!("ATHENA-{}", groups.join("-")))
}

/// Como a chave foi digitada (minúsculas, espaços, traços, O no lugar de 0) → forma canônica.
fn normalize_code(code: &str) -> Option<String> {
    let mut clean: String = code.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect();
    if clean.len() == 38 && clean.starts_with("ATHENA") {
        clean.drain(..6);
    }
    let clean: String = clean.chars().map(|c| match c { 'O' => '0', 'I' | 'L' => '1', other => other }).collect();
    (clean.len() == 32 && clean.bytes().all(|b| RECOVERY_ALPHABET.contains(&b))).then_some(clean)
}

/// O item pode ser recuperado (chave global ou a chave própria das versões anteriores)?
pub fn has_recovery(target: &Target) -> bool {
    read_target(target).is_ok_and(|(lock, _)| lock.global.is_some() || lock.recovery.is_some())
}

/// Senha esquecida: com a chave de recuperação (global, ou a própria das versões anteriores),
/// define uma senha nova; a recuperação continua valendo.
pub fn recover(target: &Target, code: &str, password: &str) -> Result<Key, String> {
    check_password(password)?;
    let (lock, body) = read_target(target)?;
    let key = match (&lock.global, &lock.recovery) {
        (None, None) => return Err("Este item não tem chave de recuperação".into()),
        (global, recovery) => global.as_ref().map(|g| unwrap_global(g, code)).filter(|r| r.is_ok())
            .or_else(|| recovery.as_ref().map(|r| unwrap(r, &normalize_code(code).unwrap_or_default(), RECOVERY_AAD, "Chave de recuperação incorreta")))
            .unwrap_or_else(|| Err("Chave de recuperação incorreta".into()))?,
    };
    write_target(target, &new_lock(&key, password, lock.recovery, lock.global)?, &body)?;
    Ok(key)
}

// ---------- Arquivo protegido sozinho ----------
const FILE_MAGIC: &[u8; 8] = b"AYLAFIL1";
/// Final dos arquivos protegidos sozinhos (o nome original continua visível antes dele).
pub const FILE_SUFFIX: &str = ".athena";
/// Tipos que podem ser protegidos sozinhos: notas, textos e livros.
const FILE_KINDS: [&str; 4] = ["md", "txt", "epub", "cbz"];

fn protectable(relative: &str) -> bool {
    Path::new(relative).extension().is_some_and(|e| FILE_KINDS.contains(&e.to_string_lossy().to_lowercase().as_str()))
}

/// `.athena` do arquivo (caminho relativo comum), se ele estiver protegido sozinho.
pub fn protected_file_of(root: &Path, relative: &str) -> Option<Target> {
    let relative = clean(relative).ok().filter(|r| protectable(r))?;
    let path = root.join(format!("{relative}{FILE_SUFFIX}"));
    path.is_file().then_some(Target::File(path))
}

/// Arquivos protegidos sozinhos da raiz (caminhos originais), fora das pastas protegidas.
pub fn protected_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(root.join(&dir)) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(kind) = entry.file_type() else { continue };
            if name.starts_with('.') || kind.is_symlink() {
                continue;
            }
            let relative = dir.join(&name);
            if kind.is_dir() {
                if !is_protected(&root.join(&relative)) { pending.push(relative); }
            } else if let Some(original) = name.strip_suffix(FILE_SUFFIX).filter(|o| protectable(o)) {
                out.push(dir.join(original).components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"));
            }
        }
    }
    out.sort();
    out
}

/// Protege um arquivo: grava `Nome.ext.athena`, confere e só então apaga o original (de vez).
pub fn protect_file(root: &Path, relative: &str, password: &str, global: &[u8; 32]) -> Result<(), String> {
    check_password(password)?;
    let relative = clean(relative)?;
    if !protectable(&relative) {
        return Err("Só notas, textos e livros podem ser protegidos sozinhos".into());
    }
    if protected_folder_of(root, &relative).is_some() {
        return Err("Este arquivo já está numa pasta protegida".into());
    }
    let original = root.join(&relative);
    let meta = fs::symlink_metadata(&original).map_err(|_| "Arquivo não encontrado")?;
    if !meta.is_file() {
        return Err("Arquivo não encontrado".into());
    }
    if meta.len() > MAX_FILE {
        return Err("Arquivos protegidos vão até 100 MB".into());
    }
    let target = Target::File(root.join(format!("{relative}{FILE_SUFFIX}")));
    if target.path().exists() {
        return Err("Já existe uma versão protegida deste arquivo".into());
    }
    let key = Key(Zeroizing::new(random::<32>()?));
    let plain = Zeroizing::new(fs::read(&original).map_err(|_| "Não foi possível ler o arquivo")?);
    let lock = new_lock(&key, password, None, Some(wrap_global(global, &key)?))?;
    write_target(&target, &lock, &seal(&key.0, FILE_MAGIC, &plain)?)?;
    if file_read(&target, &key).map(|back| back.as_slice() == plain.as_slice()) != Ok(true) {
        let _ = fs::remove_file(target.path());
        return Err("Falha ao conferir a cópia cifrada".into());
    }
    fs::remove_file(&original).map_err(|_| "O arquivo foi protegido, mas o original não pôde ser apagado".into())
}

/// Conteúdo de um arquivo protegido sozinho.
pub fn file_read(target: &Target, key: &Key) -> Result<Vec<u8>, String> {
    let (_, body) = read_target(target)?;
    open(&key.0, FILE_MAGIC, &body).ok_or_else(|| "Arquivo protegido danificado".into())
}

/// Data de modificação (segundos) do arquivo protegido no disco.
pub fn file_modified(target: &Target) -> i64 {
    fs::metadata(target.path()).ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64)
}

/// Grava o conteúdo novo (mesma proteção); recusa se mudou desde `expected`.
pub fn file_write(target: &Target, key: &Key, plain: &[u8], expected: Option<i64>) -> Result<i64, String> {
    if plain.len() as u64 > MAX_FILE {
        return Err("Arquivos protegidos vão até 100 MB".into());
    }
    if expected.is_some_and(|e| file_modified(target) > e) {
        return Err("Esta nota mudou fora da Ayla. Ela foi recarregada; refaça a última alteração.".into());
    }
    let (lock, _) = read_target(target)?;
    write_target(target, &lock, &seal(&key.0, FILE_MAGIC, plain)?)?;
    Ok(file_modified(target))
}

/// Tira a proteção de um arquivo: volta a ser comum, no mesmo lugar.
pub fn unprotect_file(target: &Target, key: &Key) -> Result<(), String> {
    let Target::File(path) = target else { return Err("Este item é uma pasta".into()) };
    let plain = Zeroizing::new(file_read(target, key)?);
    let original = PathBuf::from(path.to_string_lossy().trim_end_matches(FILE_SUFFIX));
    if original.exists() {
        return Err("Já existe um arquivo comum com esse nome; renomeie antes".into());
    }
    write_atomic(&original, &plain)?;
    fs::remove_file(path).map_err(|_| "Não foi possível tirar a proteção".into())
}

/// Protege uma pasta: cifra cada arquivo e subpasta, confere tudo e só então apaga os originais.
/// Arquivos e pastas ocultos (como `.obsidian`) ficam como estão.
/// Devolve quantos arquivos foram cifrados. A chave dos dados também fica cifrada para a chave de
/// recuperação global (`global`, a parte pública).
pub fn protect(root: &Path, folder: &str, password: &str, global: &[u8; 32]) -> Result<usize, String> {
    let folder = clean(folder)?;
    let dir = root.join(&folder);
    if !dir.is_dir() {
        return Err("Pasta não encontrada".into());
    }
    if protected_folder_of(root, &folder).is_some() {
        return Err("Esta pasta já está protegida".into());
    }
    if !protected_folders(&dir).is_empty() {
        return Err("Há uma pasta protegida dentro desta; proteja só uma delas".into());
    }
    if !protected_files(&dir).is_empty() {
        return Err("Há arquivos protegidos nesta pasta; tire a proteção deles antes de proteger a pasta".into());
    }
    if password.chars().count() < MIN_PASSWORD {
        return Err(format!("Use uma senha com pelo menos {MIN_PASSWORD} caracteres"));
    }
    let (mut files, mut folders) = (Vec::new(), Vec::new());
    let mut pending = vec![PathBuf::new()];
    while let Some(sub) = pending.pop() {
        let entries = fs::read_dir(dir.join(&sub)).map_err(|_| "Não foi possível ler a pasta")?;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(kind) = entry.file_type() else { continue };
            if name.starts_with('.') || kind.is_symlink() {
                continue;
            }
            let relative = sub.join(&name);
            if kind.is_dir() {
                folders.push(relative.clone());
                pending.push(relative);
            } else if kind.is_file() {
                if entry.metadata().map_or(0, |m| m.len()) > MAX_FILE {
                    return Err(format!("{} passa de 100 MB, o limite das pastas protegidas", relative.to_string_lossy()));
                }
                files.push(relative);
            }
        }
    }
    let key = Key(Zeroizing::new(random::<32>()?));
    let mut created = Vec::new();
    let undo = |created: &[PathBuf]| for file in created { let _ = fs::remove_file(file); };
    let as_text = |path: &Path| path.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
    for sub in &folders {
        let id = hex(&random::<16>()?);
        let entry = Entry { path: as_text(sub), kind: "folder".into(), modified: now(), bytes: 0, id: id.clone() };
        let result = encode(&key, &id, &entry, &[]).and_then(|data| write_atomic(&file_of(&dir, &id), &data));
        if let Err(error) = result { undo(&created); return Err(error); }
        created.push(file_of(&dir, &id));
    }
    for file in &files {
        let result = (|| {
            let body = Zeroizing::new(fs::read(dir.join(file)).map_err(|_| format!("Não foi possível ler {}", as_text(file)))?);
            let modified = fs::metadata(dir.join(file)).ok().and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(now(), |d| d.as_secs() as i64);
            let id = hex(&random::<16>()?);
            let entry = Entry { path: as_text(file), kind: "file".into(), modified, bytes: body.len() as u64, id: id.clone() };
            let target = file_of(&dir, &id);
            write_atomic(&target, &encode(&key, &id, &entry, &body)?)?;
            created.push(target);
            // Conferência: decifra o que acabou de gravar antes de apagar qualquer original.
            let (_, offset) = read_header(&key, &file_of(&dir, &id)).ok_or("Falha ao conferir a cópia cifrada")?;
            let sealed = fs::read(file_of(&dir, &id)).map_err(|_| "Falha ao conferir a cópia cifrada")?;
            let back = Zeroizing::new(open(&key.0, id.as_bytes(), &sealed[offset as usize..]).ok_or("Falha ao conferir a cópia cifrada")?);
            if back.as_slice() != body.as_slice() { return Err("Falha ao conferir a cópia cifrada".to_string()); }
            Ok(())
        })();
        if let Err(error) = result { undo(&created); return Err(error); }
    }
    let saved = new_lock(&key, password, None, Some(wrap_global(global, &key)?)).and_then(|lock| write_target(&Target::Folder(dir.clone()), &lock, &[]));
    if let Err(error) = saved { undo(&created); return Err(error); }
    // Tudo cifrado e conferido: agora sim os originais saem (de vez; ir para a lixeira os deixaria legíveis).
    for file in &files {
        let _ = fs::remove_file(dir.join(file));
    }
    folders.sort_by_key(|f| std::cmp::Reverse(f.components().count()));
    for sub in &folders {
        let _ = fs::remove_dir(dir.join(sub));
    }
    Ok(files.len())
}

/// Tira a proteção: decifra tudo de volta para arquivos comuns e apaga os cifrados e a proteção.
pub fn unprotect(dir: &Path, key: &Key) -> Result<usize, String> {
    let all = entries(dir, key);
    for entry in &all {
        if dir.join(&entry.path).exists() {
            return Err(format!("Já existe {} fora da proteção; renomeie antes", entry.path));
        }
    }
    let mut restored = 0;
    for entry in all.iter().filter(|e| e.kind == "folder") {
        fs::create_dir_all(dir.join(&entry.path)).map_err(|_| "Não foi possível recriar as pastas")?;
    }
    for entry in all.iter().filter(|e| e.kind == "file") {
        let (body, _) = read(dir, key, &entry.path)?;
        let target = dir.join(&entry.path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|_| "Não foi possível recriar as pastas")?;
        }
        write_atomic(&target, &body)?;
        restored += 1;
    }
    for entry in &all {
        let _ = fs::remove_file(file_of(dir, &entry.id));
    }
    fs::remove_file(dir.join(LOCK_FILE)).map_err(|_| "Não foi possível tirar a proteção")?;
    Ok(restored)
}

/// Pastas desbloqueadas nesta execução (caminho absoluto → chave e momento da senha). Ao fechar a
/// Ayla, tudo volta a ficar bloqueado.
static SESSIONS: Mutex<Option<HashMap<PathBuf, (Key, i64)>>> = Mutex::new(None);

pub fn start_session(dir: &Path, key: Key) -> i64 {
    let at = now();
    if let Ok(mut map) = SESSIONS.lock() {
        map.get_or_insert_with(HashMap::new).insert(dir.to_path_buf(), (key, at));
    }
    at
}

pub fn session(dir: &Path) -> Option<(Key, i64)> {
    SESSIONS.lock().ok()?.as_ref()?.get(dir).cloned()
}

pub fn end_session(dir: &Path) {
    if let Ok(mut map) = SESSIONS.lock() {
        if let Some(map) = map.as_mut() {
            map.remove(dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ayla-lock-{}", hex(&random::<8>().unwrap())));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn protege_cifra_nomes_e_conteudo_e_devolve_tudo_ao_tirar_a_protecao() {
        let root = temp();
        let dir = root.join("Senhas");
        fs::create_dir_all(dir.join("Bancos/Vazia")).unwrap();
        fs::create_dir_all(dir.join(".obsidian")).unwrap();
        fs::write(dir.join("Email.md"), "senha: hunter2-secreta").unwrap();
        fs::write(dir.join("Bancos/Itaú.md"), "agência 1234").unwrap();
        fs::write(dir.join("Bancos/cartão.png"), b"\x89PNG\r\n\x1a\nimagem").unwrap();
        fs::write(dir.join(".obsidian/app.json"), "{}").unwrap();
        let (code, public) = new_global().unwrap();
        assert!(code.starts_with("ATHENA-") && code.len() == 7 + 32 + 7, "{code}");
        assert_eq!(global_public(&code.to_lowercase()).unwrap(), public, "o código sempre gera a mesma chave pública");
        assert!(protect(&root, "Senhas", "curta", &public).is_err(), "senha curta");
        assert_eq!(protect(&root, "Senhas", "senha forte 123", &public).unwrap(), 3);
        assert!(protect(&root, "Senhas", "senha forte 123", &public).is_err(), "já protegida");
        let target = Target::Folder(dir.clone());
        assert_eq!(protected_folders(&root), ["Senhas"]);
        assert_eq!(protected_folder_of(&root, "Senhas/Bancos/Itaú.md").as_deref(), Some("Senhas"));
        assert_eq!(protected_folder_of(&root, "Outra/nota.md"), None);

        // No disco: só a proteção, os cifrados (nomes aleatórios) e o que era oculto.
        let mut names: Vec<String> = fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        names.sort();
        assert!(names.contains(&".athena-lock.json".to_string()) && names.contains(&".obsidian".to_string()));
        assert_eq!(names.iter().filter(|n| n.ends_with(".athena")).count(), 5, "3 arquivos e 2 subpastas");
        assert!(!dir.join("Bancos").exists() && !dir.join("Email.md").exists());
        for name in names.iter().filter(|n| n.ends_with(".athena")) {
            let raw = fs::read(dir.join(name)).unwrap();
            let text = String::from_utf8_lossy(&raw);
            assert!(!text.contains("hunter2") && !text.contains("Itaú") && !text.contains("Email") && !text.contains("Bancos"), "{name} vaza conteúdo");
        }
        let lock = fs::read_to_string(dir.join(LOCK_FILE)).unwrap();
        assert!(!lock.contains("senha forte") && lock.contains("\"global\""));

        assert_eq!(unlock(&target, "senha errada").err().as_deref(), Some("Senha incorreta"));
        let key = unlock(&target, "senha forte 123").unwrap();
        let list: Vec<(String, String)> = entries(&dir, &key).into_iter().map(|e| (e.path, e.kind)).collect();
        assert_eq!(list, [
            ("Bancos".into(), "folder".into()), ("Bancos/Itaú.md".into(), "file".into()), ("Bancos/Vazia".into(), "folder".into()),
            ("Bancos/cartão.png".into(), "file".into()), ("Email.md".into(), "file".into()),
        ]);
        assert_eq!(read(&dir, &key, "Email.md").unwrap().0, b"senha: hunter2-secreta");

        // Editar, criar, renomear e apagar sem sair da proteção.
        let (_, entry) = read(&dir, &key, "Email.md").unwrap();
        let saved = write(&dir, &key, "Email.md", b"nova", Some(entry.modified)).unwrap();
        assert!(write(&dir, &key, "Email.md", b"antiga", Some(saved.modified - 10)).is_err(), "conflito");
        write(&dir, &key, "Cofres/Casa.md", b"chave", None).unwrap();
        assert!(entries(&dir, &key).iter().any(|e| e.path == "Cofres" && e.kind == "folder"));
        rename(&dir, &key, "Bancos", "Finanças/Bancos").unwrap();
        assert_eq!(read(&dir, &key, "Finanças/Bancos/Itaú.md").unwrap().0, "agência 1234".as_bytes());
        assert!(rename(&dir, &key, "Email.md", "Cofres/Casa.md").is_err(), "não sobrescreve");
        assert!(read(&dir, &key, "../fora.md").is_err());
        trash(&dir, &key, "Cofres", &root.join(".trash")).unwrap();
        assert!(entries(&dir, &key).iter().all(|e| !e.path.starts_with("Cofres")));
        assert_eq!(fs::read_dir(root.join(".trash")).unwrap().count(), 2, "vai cifrado para a lixeira");

        change_password(&target, "senha forte 123", "outra senha boa").unwrap();
        assert!(unlock(&target, "senha forte 123").is_err());
        assert!(unlock(&target, "outra senha boa").is_ok());

        // Senha esquecida: a chave global (digitada de qualquer jeito) define outra e continua valendo.
        assert!(!lock.contains(&code[7..]), "a chave não fica no disco");
        assert!(has_recovery(&target));
        let (other, other_public) = new_global().unwrap();
        assert_eq!(recover(&target, &other, "nova senha 1").err().as_deref(), Some("Chave de recuperação incorreta"), "chave global de outra instalação");
        assert!(recover(&target, "curta", "nova senha 1").is_err());
        let typed = code.to_lowercase().replace('-', " ").replace('0', "o");
        assert!(recover(&target, &typed, "curta").is_err(), "senha nova curta");
        let recovered = recover(&target, &typed, "nova senha 1").unwrap();
        assert_eq!(read(&dir, &recovered, "Email.md").unwrap().0, b"nova");
        assert!(unlock(&target, "outra senha boa").is_err());
        let key = unlock(&target, "nova senha 1").unwrap();
        recover(&target, &code, "nova senha 2").unwrap();
        // Chave global trocada: ao abrir o item, ele passa a ter a cópia da nova; a antiga deixa de valer.
        assert!(ensure_global(&target, &key, &other_public).unwrap());
        assert!(!ensure_global(&target, &key, &other_public).unwrap(), "nada a mudar");
        assert!(recover(&target, &code, "nova senha 3").is_err());
        recover(&target, &other, "outra senha boa").unwrap();
        drop(key);
        let key = unlock(&target, "outra senha boa").unwrap();
        assert_eq!(read(&dir, &key, "Email.md").unwrap().0, b"nova");

        assert_eq!(unprotect(&dir, &key).unwrap(), 3);
        assert_eq!(fs::read_to_string(dir.join("Email.md")).unwrap(), "nova");
        assert_eq!(fs::read(dir.join("Finanças/Bancos/cartão.png")).unwrap(), b"\x89PNG\r\n\x1a\nimagem");
        assert!(dir.join("Finanças/Bancos/Vazia").is_dir());
        assert!(!is_protected(&dir));
        assert!(fs::read_dir(&dir).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().ends_with(".athena")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn arquivo_protegido_sozinho_mantem_o_nome_e_cifra_o_conteudo() {
        let root = temp();
        fs::create_dir_all(root.join("Pessoal")).unwrap();
        fs::write(root.join("Pessoal/Senhas.md"), "banco: hunter2").unwrap();
        fs::write(root.join("Pessoal/foto.png"), b"png").unwrap();
        fs::write(root.join("Livro.epub"), b"PK livro").unwrap();
        let (code, public) = new_global().unwrap();
        assert!(protect_file(&root, "Pessoal/foto.png", "senha forte 123", &public).is_err(), "só notas, textos e livros");
        assert!(protect_file(&root, "Pessoal/Senhas.md", "curta", &public).is_err());
        protect_file(&root, "Pessoal/Senhas.md", "senha forte 123", &public).unwrap();
        protect_file(&root, "Livro.epub", "outra senha boa", &public).unwrap();
        assert!(!root.join("Pessoal/Senhas.md").exists(), "o original sai");
        let raw = fs::read(root.join("Pessoal/Senhas.md.athena")).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("hunter2"));
        assert_eq!(protected_files(&root), ["Livro.epub", "Pessoal/Senhas.md"]);
        let target = protected_file_of(&root, "Pessoal/Senhas.md").unwrap();
        assert!(protected_file_of(&root, "Pessoal/Outra.md").is_none());
        assert!(protect_file(&root, "Pessoal/Senhas.md", "senha forte 123", &public).is_err(), "já protegido");

        assert!(unlock(&target, "senha errada").is_err());
        let key = unlock(&target, "senha forte 123").unwrap();
        assert_eq!(file_read(&target, &key).unwrap(), b"banco: hunter2");
        let stamp = file_modified(&target);
        file_write(&target, &key, b"banco: novo", Some(stamp)).unwrap();
        assert!(file_write(&target, &key, b"x", Some(stamp - 10)).is_err(), "conflito");
        assert_eq!(file_read(&target, &key).unwrap(), b"banco: novo");
        change_password(&target, "senha forte 123", "nova senha 1").unwrap();
        assert_eq!(file_read(&target, &unlock(&target, "nova senha 1").unwrap()).unwrap(), b"banco: novo", "senha nova não recifra o conteúdo");
        let recovered = recover(&target, &code, "nova senha 2").unwrap();
        assert_eq!(file_read(&target, &recovered).unwrap(), b"banco: novo");
        // Conteúdo alterado no disco não decifra.
        let mut tampered = fs::read(target.path()).unwrap();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        fs::write(root.join("copia.md.athena"), &tampered).unwrap();
        assert!(file_read(&Target::File(root.join("copia.md.athena")), &recovered).is_err());

        unprotect_file(&target, &recovered).unwrap();
        assert_eq!(fs::read_to_string(root.join("Pessoal/Senhas.md")).unwrap(), "banco: novo");
        assert!(!target.path().exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parametros_reais_do_argon2_sao_validos_e_rapidos() {
        let start = std::time::Instant::now();
        let key = derive("senha forte 123", &[7; 16], 64 * 1024, 3, 1).unwrap();
        let elapsed = start.elapsed();
        assert_ne!(*key, [0; 32]);
        assert_eq!(*key, *derive("senha forte 123", &[7; 16], 64 * 1024, 3, 1).unwrap());
        eprintln!("Argon2id 64 MiB x3: {elapsed:?}");
        assert!(elapsed < std::time::Duration::from_secs(3));
    }

    #[test]
    fn arquivo_trocado_ou_alterado_nao_decifra() {
        let root = temp();
        fs::create_dir_all(root.join("P")).unwrap();
        fs::write(root.join("P/a.md"), "A").unwrap();
        fs::write(root.join("P/b.md"), "B").unwrap();
        let (code, public) = new_global().unwrap();
        protect(&root, "P", "senha forte 123", &public).unwrap();
        let dir = root.join("P");
        let target = Target::Folder(dir.clone());
        let key = unlock(&target, "senha forte 123").unwrap();
        let list = entries(&dir, &key);
        let (a, b) = (&list[0], &list[1]);
        // Conteúdo de um arquivo colado no outro: o identificador não confere.
        let raw_a = fs::read(file_of(&dir, &a.id)).unwrap();
        fs::write(file_of(&dir, &b.id), &raw_a).unwrap();
        assert!(entries(&dir, &key).iter().all(|e| e.id != b.id), "cabeçalho de outro arquivo é recusado");
        let mut tampered = raw_a.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        fs::write(file_of(&dir, &a.id), tampered).unwrap();
        assert!(read(&dir, &key, "a.md").is_err());
        assert!(protect(&root, "", "senha forte 123", &public).is_err());
        // Item protegido antes da chave global (versão anterior, com a chave própria dele).
        let mut lock: serde_json::Value = serde_json::from_slice(&fs::read(dir.join(LOCK_FILE)).unwrap()).unwrap();
        lock.as_object_mut().unwrap().remove("global");
        fs::write(dir.join(LOCK_FILE), serde_json::to_vec(&lock).unwrap()).unwrap();
        assert!(!has_recovery(&target));
        assert_eq!(recover(&target, &code, "nova senha 1").err().as_deref(), Some("Este item não tem chave de recuperação"));
        let legacy = recovery_code().unwrap();
        lock["recovery"] = serde_json::to_value(wrap(&normalize_code(&legacy).unwrap(), &key, RECOVERY_AAD).unwrap()).unwrap();
        fs::write(dir.join(LOCK_FILE), serde_json::to_vec(&lock).unwrap()).unwrap();
        recover(&target, &legacy, "nova senha 1").unwrap();
        // Ao abrir, ganha a cópia global; as duas chaves passam a valer.
        assert!(ensure_global(&target, &key, &public).unwrap());
        recover(&target, &code, "nova senha 2").unwrap();
        recover(&target, &legacy, "nova senha 3").unwrap();
        assert!(unlock(&target, "nova senha 3").is_ok());
        let _ = fs::remove_dir_all(root);
    }
}
