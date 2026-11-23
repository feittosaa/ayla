//! Athena como bloco de notas: abre, edita e salva qualquer arquivo de texto do computador, numa
//! janela própria, com a sessão (abas e rascunhos não salvos) guardada nos dados da Ayla.
//! A Ayla só lê e grava os arquivos que a pessoa escolheu: pelo seletor, pelo "Abrir com" do
//! sistema ou por uma sessão anterior.
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::UNIX_EPOCH,
};

/// Maior arquivo aberto no editor.
pub const MAX_BYTES: u64 = 20 * 1024 * 1024;
/// Maior sessão guardada (abas e rascunhos).
pub const MAX_SESSION: usize = 64 * 1024 * 1024;

#[derive(Default)]
pub struct Pad {
    allowed: Mutex<HashSet<PathBuf>>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Document {
    pub path: String,
    pub name: String,
    pub text: String,
    /// "UTF-8" ou "ISO-8859-1" (Latin-1).
    pub encoding: &'static str,
    /// Fim de linha do arquivo: "\n" ou "\r\n".
    pub eol: &'static str,
    pub modified: i64,
    pub readonly: bool,
}

pub fn modified(path: &Path) -> Option<i64> {
    let time = fs::metadata(path).ok()?.modified().ok()?;
    Some(time.duration_since(UNIX_EPOCH).ok()?.as_millis() as i64)
}

/// Texto do arquivo: UTF-8 (com ou sem BOM) ou, se não for, Latin-1. Binários são recusados.
pub fn decode(bytes: &[u8]) -> Result<(String, &'static str), String> {
    if bytes.contains(&0) {
        return Err("Este arquivo não parece ser texto".into());
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok((text.to_string(), "UTF-8")),
        Err(_) => Ok((bytes.iter().map(|&b| b as char).collect(), "ISO-8859-1")),
    }
}

pub fn encode(text: &str, encoding: &str) -> Result<Vec<u8>, String> {
    match encoding {
        "UTF-8" => Ok(text.as_bytes().to_vec()),
        "ISO-8859-1" => text.chars().map(|c| u8::try_from(c as u32).map_err(|_| format!("O caractere “{c}” não existe em ISO-8859-1. Salve em UTF-8."))).collect(),
        _ => Err("Codificação não suportada".into()),
    }
}

/// O editor trabalha com "\n"; o arquivo volta com o fim de linha escolhido.
pub fn line_ending(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

pub fn read(path: &Path) -> Result<Document, String> {
    let meta = fs::metadata(path).map_err(|_| "Arquivo não encontrado")?;
    if !meta.is_file() {
        return Err("Escolha um arquivo, não uma pasta".into());
    }
    if meta.len() > MAX_BYTES {
        return Err("Arquivo grande demais para o editor (até 20 MB)".into());
    }
    let bytes = fs::read(path).map_err(|_| "Não foi possível ler o arquivo")?;
    let (raw, encoding) = decode(&bytes)?;
    let eol = line_ending(&raw);
    Ok(Document {
        path: path.to_string_lossy().into_owned(),
        name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        text: raw.replace("\r\n", "\n"),
        encoding,
        eol,
        modified: modified(path).unwrap_or(0),
        readonly: meta.permissions().readonly(),
    })
}

/// Grava por cima com segurança (arquivo temporário na mesma pasta + troca), mantendo as
/// permissões. Recusa se o arquivo mudou no disco desde `expected` (a interface pergunta antes).
pub fn write(path: &Path, text: &str, encoding: &str, eol: &str, expected: Option<i64>) -> Result<i64, String> {
    if let (Some(expected), Some(current)) = (expected, modified(path)) {
        if expected != current {
            return Err("O arquivo mudou no disco depois que foi aberto".into());
        }
    }
    let eol = if eol == "\r\n" { "\r\n" } else { "\n" };
    let bytes = encode(&text.replace("\r\n", "\n").replace('\n', eol), encoding)?;
    let dir = path.parent().ok_or("Caminho inválido")?;
    let name = path.file_name().ok_or("Caminho inválido")?.to_string_lossy().into_owned();
    let temp = dir.join(format!(".{name}.athena-{}", std::process::id()));
    let result = (|| {
        let mut file = fs::File::create(&temp).map_err(|_| "Sem permissão para salvar nesta pasta")?;
        file.write_all(&bytes).map_err(|_| "Não foi possível salvar o arquivo")?;
        file.sync_all().map_err(|_| "Não foi possível salvar o arquivo")?;
        if let Ok(meta) = fs::metadata(path) {
            let _ = fs::set_permissions(&temp, meta.permissions());
        }
        fs::rename(&temp, path).map_err(|_| "Não foi possível salvar o arquivo".to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result?;
    modified(path).ok_or_else(|| "Arquivo salvo, mas não foi possível conferir a data".into())
}

impl Pad {
    pub fn allow(&self, path: &Path) {
        if let Ok(mut allowed) = self.allowed.lock() {
            allowed.insert(path.to_path_buf());
        }
    }
    /// Caminho liberado; o arquivo pode ainda não existir (Salvar como).
    pub fn check(&self, path: &str) -> Result<PathBuf, String> {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err("Caminho inválido".into());
        }
        let allowed = self.allowed.lock().map_err(|_| "Editor ocupado")?;
        allowed.contains(&path).then_some(path).ok_or_else(|| "Abra o arquivo pela Athena antes de editá-lo".into())
    }
    /// Arquivos citados numa sessão guardada pela própria Ayla voltam a ser editáveis.
    pub fn allow_session(&self, session: &Value) {
        for tab in session["tabs"].as_array().into_iter().flatten() {
            if let Some(path) = tab["path"].as_str().filter(|p| Path::new(p).is_absolute()) {
                self.allow(Path::new(path));
            }
        }
        for path in session["recent"].as_array().into_iter().flatten().filter_map(|p| p.as_str()) {
            if Path::new(path).is_absolute() {
                self.allow(Path::new(path));
            }
        }
    }
}

pub fn session_path(root: &Path) -> PathBuf {
    root.join("athena-bloco").join("sessao.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_only_text_and_notes() {
        for ok in ["/a/nota.md", "/a/LISTA.TXT", "/a/x.markdown"] { assert!(openable(Path::new(ok)), "{ok}"); }
        for no in ["/a/foto.png", "/a/script.py", "/a/sem-extensao", "/a/.md"] { assert!(!openable(Path::new(no)), "{no}"); }
    }

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ayla-pad-{}", super::super::store::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn keeps_encoding_and_line_endings_and_refuses_binaries() {
        let dir = temp();
        let file = dir.join("antigo.txt");
        fs::write(&file, b"ol\xe1\r\nmundo\r\n").unwrap();
        let doc = read(&file).unwrap();
        assert_eq!((doc.text.as_str(), doc.encoding, doc.eol), ("olá\nmundo\n", "ISO-8859-1", "\r\n"));
        let stamp = write(&file, "olá\nmundo novo\n", doc.encoding, doc.eol, Some(doc.modified)).unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"ol\xe1\r\nmundo novo\r\n");
        assert!(write(&file, "x", "ISO-8859-1", "\n", Some(stamp - 5000)).is_err(), "mudou no disco");
        assert!(encode("ação 😀", "ISO-8859-1").is_err());
        fs::write(dir.join("bin"), b"a\0b").unwrap();
        assert!(read(&dir.join("bin")).is_err());
        fs::write(dir.join("bom.md"), "\u{feff}# título\n").unwrap();
        assert_eq!(read(&dir.join("bom.md")).unwrap().text, "# título\n");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_files_the_person_chose_are_editable() {
        let pad = Pad::default();
        assert!(pad.check("/etc/passwd").is_err());
        pad.allow(Path::new("/tmp/nota.txt"));
        assert!(pad.check("/tmp/nota.txt").is_ok());
        assert!(pad.check("relativo.txt").is_err());
        pad.allow_session(&serde_json::json!({"tabs":[{"path":"/tmp/a.md"},{"path":null,"draft":"x"}],"recent":["/tmp/b.txt","rel"]}));
        assert!(pad.check("/tmp/a.md").is_ok() && pad.check("/tmp/b.txt").is_ok());
    }
}

// Comandos da janela da Athena.
use super::services::Core;
use tauri_plugin_dialog::DialogExt;

fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> impl std::future::Future<Output = Result<T, String>> {
    async move { tauri::async_runtime::spawn_blocking(work).await.map_err(|_| "Falha no editor".to_string()) }
}

/// Seletor de arquivos: qualquer texto. Os escolhidos ficam liberados para leitura e gravação.
/// Extensões oferecidas pelo seletor "Abrir".
const OPEN_EXTENSIONS: &[&str] = &["txt", "md", "markdown"];
fn openable(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| OPEN_EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

#[tauri::command]
pub async fn pad_open_dialog(app: tauri::AppHandle, pad: tauri::State<'_, Pad>) -> Result<Vec<String>, String> {
    let dialog = app.clone();
    // Só textos e notas (.txt e .md) aparecem no seletor; o filtro se repete aqui por garantia.
    let chosen = blocking(move || dialog.dialog().file().set_title("Abrir").add_filter("Textos e notas", OPEN_EXTENSIONS).blocking_pick_files()).await?.unwrap_or_default();
    let paths: Vec<PathBuf> = chosen.into_iter().filter_map(|p| p.into_path().ok()).filter(|p| openable(p)).collect();
    for path in &paths { pad.allow(path); }
    Ok(paths.into_iter().map(|p| p.to_string_lossy().into_owned()).collect())
}

/// "Salvar como": devolve o caminho escolhido (liberado) ou `None` se a pessoa cancelou.
#[tauri::command]
pub async fn pad_save_dialog(app: tauri::AppHandle, pad: tauri::State<'_, Pad>, name: String, folder: Option<String>) -> Result<Option<String>, String> {
    let dialog = app.clone();
    let chosen = blocking(move || {
        let mut picker = dialog.dialog().file().set_title("Salvar como").set_file_name(&name);
        if let Some(folder) = folder.filter(|f| Path::new(f).is_dir()) { picker = picker.set_directory(folder); }
        picker.blocking_save_file()
    }).await?;
    let Some(path) = chosen.and_then(|p| p.into_path().ok()) else { return Ok(None) };
    pad.allow(&path);
    Ok(Some(path.to_string_lossy().into_owned()))
}

#[tauri::command]
pub async fn pad_read(pad: tauri::State<'_, Pad>, path: String) -> Result<Document, String> {
    let path = pad.check(&path)?;
    blocking(move || read(&path)).await?
}

#[tauri::command]
pub async fn pad_write(pad: tauri::State<'_, Pad>, path: String, text: String, encoding: String, eol: String, expected: Option<i64>) -> Result<i64, String> {
    let path = pad.check(&path)?;
    blocking(move || write(&path, &text, &encoding, &eol, expected)).await?
}

/// Data de modificação no disco, para avisar quando o arquivo mudou fora do editor.
#[tauri::command]
pub fn pad_modified(pad: tauri::State<'_, Pad>, path: String) -> Result<Option<i64>, String> {
    Ok(modified(&pad.check(&path)?))
}

/// Arquivos pedidos pelo "Abrir com" do sistema (cada pedido uma vez), já liberados.
#[tauri::command]
pub fn pad_requests(pad: tauri::State<'_, Pad>) -> Vec<String> {
    let paths = crate::desktop::athena_requests();
    for path in &paths { pad.allow(Path::new(path)); }
    paths
}

#[tauri::command]
pub async fn pad_session_load(core: tauri::State<'_, Core>, pad: tauri::State<'_, Pad>) -> Result<Value, String> {
    let path = session_path(&core.root);
    let session: Value = blocking(move || fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null)).await?;
    pad.allow_session(&session);
    Ok(session)
}

/// Guarda abas, rascunhos não salvos, recentes e preferências do editor (como o Text Editor).
#[tauri::command]
pub async fn pad_session_save(core: tauri::State<'_, Core>, session: Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(&session).map_err(|_| "Sessão inválida")?;
    if bytes.len() > MAX_SESSION { return Err("Rascunhos grandes demais para guardar".into()); }
    let path = session_path(&core.root);
    blocking(move || {
        let dir = path.parent().ok_or("Caminho inválido")?;
        fs::create_dir_all(dir).map_err(|_| "Não foi possível guardar a sessão")?;
        let temp = dir.join("sessao.json.part");
        fs::write(&temp, bytes).and_then(|_| fs::rename(&temp, &path)).map_err(|_| "Não foi possível guardar a sessão".to_string())
    }).await?
}
