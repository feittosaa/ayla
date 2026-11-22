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

fn lock_file(dir: &Path) -> Result<LockFile, String> {
    let text = fs::read(dir.join(LOCK_FILE)).map_err(|_| "Esta pasta não está protegida")?;
    let lock: LockFile = serde_json::from_slice(&text).map_err(|_| "Proteção da pasta danificada")?;
    if lock.version != 1 || lock.kdf != "argon2id" {
        return Err("Proteção da pasta em formato desconhecido".into());
    }
    Ok(lock)
}

fn write_lock(dir: &Path, lock: &LockFile) -> Result<(), String> {
    write_atomic(&dir.join(LOCK_FILE), &serde_json::to_vec_pretty(lock).map_err(|_| "Falha ao salvar a proteção")?)
}

fn wrap(secret: &str, key: &Key, aad: &[u8]) -> Result<Wrapped, String> {
    let salt = random::<16>()?;
    let kek = derive(secret, &salt, MEMORY_KIB, ITERATIONS, 1)?;
    Ok(Wrapped { memory: MEMORY_KIB, iterations: ITERATIONS, parallelism: 1, salt: B64.encode(salt), key: B64.encode(seal(&kek, aad, key.0.as_slice())?) })
}

fn unwrap(wrapped: &Wrapped, secret: &str, aad: &[u8], wrong: &str) -> Result<Key, String> {
    if wrapped.memory > 1024 * 1024 || wrapped.iterations > 20 || wrapped.parallelism > 8 {
        return Err("Proteção da pasta em formato desconhecido".into());
    }
    let salt = B64.decode(&wrapped.salt).map_err(|_| "Proteção da pasta danificada")?;
    let sealed = B64.decode(&wrapped.key).map_err(|_| "Proteção da pasta danificada")?;
    let kek = derive(secret, &salt, wrapped.memory, wrapped.iterations, wrapped.parallelism)?;
    let plain = Zeroizing::new(open(&kek, aad, &sealed).ok_or(wrong)?);
    let key: [u8; 32] = plain.as_slice().try_into().map_err(|_| "Proteção da pasta danificada")?;
    Ok(Key(Zeroizing::new(key)))
}

fn main_key(lock: &LockFile) -> Wrapped {
    Wrapped { memory: lock.memory, iterations: lock.iterations, parallelism: lock.parallelism, salt: lock.salt.clone(), key: lock.key.clone() }
}

/// Confere a senha e devolve a chave dos dados.
pub fn unlock(dir: &Path, password: &str) -> Result<Key, String> {
    let lock = lock_file(dir)?;
    unwrap(&main_key(&lock), password, MAGIC, "Senha incorreta")
}

/// Grava a proteção com a senha nova, mantendo a chave de recuperação indicada.
fn save_lock(dir: &Path, key: &Key, password: &str, recovery: Option<Wrapped>) -> Result<(), String> {
    if password.chars().count() < MIN_PASSWORD {
        return Err(format!("Use uma senha com pelo menos {MIN_PASSWORD} caracteres"));
    }
    let main = wrap(password, key, MAGIC)?;
    write_lock(dir, &LockFile {
        version: 1, kdf: "argon2id".into(), memory: main.memory, iterations: main.iterations, parallelism: main.parallelism,
        salt: main.salt, key: main.key, recovery,
    })
}

/// Troca a senha (só a chave dos dados é recifrada); a chave de recuperação continua valendo.
pub fn change_password(dir: &Path, current: &str, new: &str) -> Result<(), String> {
    let lock = lock_file(dir)?;
    let key = unwrap(&main_key(&lock), current, MAGIC, "Senha incorreta")?;
    save_lock(dir, &key, new, lock.recovery)
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

/// A pasta tem chave de recuperação?
pub fn has_recovery(dir: &Path) -> bool {
    lock_file(dir).is_ok_and(|lock| lock.recovery.is_some())
}

/// Cria uma chave de recuperação nova (a anterior deixa de valer). Pede a senha atual.
pub fn new_recovery(dir: &Path, password: &str) -> Result<String, String> {
    let mut lock = lock_file(dir)?;
    let key = unwrap(&main_key(&lock), password, MAGIC, "Senha incorreta")?;
    let code = recovery_code()?;
    lock.recovery = Some(wrap(&normalize_code(&code).ok_or("Falha ao criar a chave")?, &key, RECOVERY_AAD)?);
    write_lock(dir, &lock)?;
    Ok(code)
}

/// Senha esquecida: com a chave de recuperação, define uma senha nova (a chave continua valendo).
pub fn recover(dir: &Path, code: &str, password: &str) -> Result<Key, String> {
    let lock = lock_file(dir)?;
    let recovery = lock.recovery.clone().ok_or("Esta pasta não tem chave de recuperação")?;
    let code = normalize_code(code).ok_or("Chave de recuperação incorreta")?;
    let key = unwrap(&recovery, &code, RECOVERY_AAD, "Chave de recuperação incorreta")?;
    save_lock(dir, &key, password, Some(recovery))?;
    Ok(key)
}

/// Protege uma pasta: cifra cada arquivo e subpasta, confere tudo e só então apaga os originais.
/// Arquivos e pastas ocultos (como `.obsidian`) ficam como estão.
/// Devolve quantos arquivos foram cifrados e a chave de recuperação (mostrada uma única vez).
pub fn protect(root: &Path, folder: &str, password: &str) -> Result<(usize, String), String> {
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
    let saved = recovery_code().and_then(|code| {
        let recovery = wrap(&normalize_code(&code).ok_or("Falha ao criar a chave")?, &key, RECOVERY_AAD)?;
        save_lock(&dir, &key, password, Some(recovery)).map(|_| code)
    });
    let code = match saved { Ok(code) => code, Err(error) => { undo(&created); return Err(error); } };
    // Tudo cifrado e conferido: agora sim os originais saem (de vez; ir para a lixeira os deixaria legíveis).
    for file in &files {
        let _ = fs::remove_file(dir.join(file));
    }
    folders.sort_by_key(|f| std::cmp::Reverse(f.components().count()));
    for sub in &folders {
        let _ = fs::remove_dir(dir.join(sub));
    }
    Ok((files.len(), code))
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
        assert!(protect(&root, "Senhas", "curta").is_err(), "senha curta");
        let (count, code) = protect(&root, "Senhas", "senha forte 123").unwrap();
        assert_eq!(count, 3);
        assert!(code.starts_with("ATHENA-") && code.len() == 7 + 32 + 7, "{code}");
        assert!(protect(&root, "Senhas", "senha forte 123").is_err(), "já protegida");
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
        assert!(!lock.contains("senha forte"));

        assert_eq!(unlock(&dir, "senha errada").err().as_deref(), Some("Senha incorreta"));
        let key = unlock(&dir, "senha forte 123").unwrap();
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

        change_password(&dir, "senha forte 123", "outra senha boa").unwrap();
        assert!(unlock(&dir, "senha forte 123").is_err());
        assert!(unlock(&dir, "outra senha boa").is_ok());

        // Senha esquecida: a chave (digitada de qualquer jeito) define outra e continua valendo.
        assert!(!lock.contains(&code[7..]), "a chave não fica no disco");
        assert!(has_recovery(&dir));
        assert_eq!(recover(&dir, "ATHENA-0000-0000-0000-0000-0000-0000-0000-0000", "nova senha 1").err().as_deref(), Some("Chave de recuperação incorreta"));
        assert!(recover(&dir, "curta", "nova senha 1").is_err());
        let typed = code.to_lowercase().replace('-', " ").replace('0', "o");
        assert!(recover(&dir, &typed, "curta").is_err(), "senha nova curta");
        let recovered = recover(&dir, &typed, "nova senha 1").unwrap();
        assert_eq!(read(&dir, &recovered, "Email.md").unwrap().0, b"nova");
        assert!(unlock(&dir, "outra senha boa").is_err());
        let key = unlock(&dir, "nova senha 1").unwrap();
        recover(&dir, &code, "nova senha 2").unwrap();
        let key2 = new_recovery(&dir, "nova senha 2").unwrap();
        assert!(recover(&dir, &code, "nova senha 3").is_err(), "a chave anterior deixa de valer");
        recover(&dir, &key2, "outra senha boa").unwrap();
        drop(key);
        let key = unlock(&dir, "outra senha boa").unwrap();
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
        protect(&root, "P", "senha forte 123").unwrap();
        let dir = root.join("P");
        let key = unlock(&dir, "senha forte 123").unwrap();
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
        assert!(protect(&root, "", "senha forte 123").is_err());
        // Pasta protegida antes da chave de recuperação existir: cria uma com a senha.
        let lock: serde_json::Value = serde_json::from_slice(&fs::read(dir.join(LOCK_FILE)).unwrap()).unwrap();
        let mut old = lock.clone();
        old.as_object_mut().unwrap().remove("recovery");
        fs::write(dir.join(LOCK_FILE), serde_json::to_vec(&old).unwrap()).unwrap();
        assert!(!has_recovery(&dir));
        assert_eq!(recover(&dir, "ATHENA-0000-0000-0000-0000-0000-0000-0000-0000", "nova senha 1").err().as_deref(), Some("Esta pasta não tem chave de recuperação"));
        assert!(new_recovery(&dir, "senha errada").is_err());
        let code = new_recovery(&dir, "senha forte 123").unwrap();
        assert!(has_recovery(&dir));
        recover(&dir, &code, "nova senha 1").unwrap();
        assert!(unlock(&dir, "nova senha 1").is_ok());
        let _ = fs::remove_dir_all(root);
    }
}
