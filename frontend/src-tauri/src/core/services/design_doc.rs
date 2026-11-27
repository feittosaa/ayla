//! Comandos dos designs sobre documentos (PDFs e imagens da Athena/Iris): abrir (com a cópia
//! intacta guardada como base), ler a base e gravar no original, com backup antes.
use super::*;
use sha2::{Digest, Sha256};

fn bases(state: &Core) -> PathBuf { state.root.join("design-bases") }

fn hash(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }

/// Tipo pelo nome: "pdf" ou "imagem" (as que dá para gravar de volta no mesmo formato).
fn tipo(name: &str) -> Option<&'static str> {
    match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("pdf") => Some("pdf"),
        Some("png" | "jpg" | "jpeg" | "jfif" | "webp") => Some("imagem"),
        _ => None,
    }
}

fn extensao(name: &str) -> String { name.rsplit('.').next().unwrap_or("").to_ascii_lowercase() }

/// Abre um PDF ou imagem para editar: guarda a cópia intacta (base) e diz o tipo, o nome e o hash
/// atual do arquivo (para saber depois se ele mudou fora do editor).
#[tauri::command]
pub async fn design_documento(state: tauri::State<'_, Core>, file: Option<String>, root: Option<String>, path: Option<String>) -> Result<Value, String> {
    let (source, name, _) = iris_source(&state, &file, &root, &path, "Arquivos de pastas protegidas não abrem nos designs")?;
    let kind = tipo(&name).ok_or("Só PDFs e imagens PNG, JPG ou WebP abrem nos designs")?;
    let dir = bases(&state);
    tauri::async_runtime::spawn_blocking(move || {
        let meta = fs::metadata(&source).map_err(|_| "Arquivo não encontrado")?;
        if meta.len() > 200 << 20 { return Err("Arquivo grande demais para editar (até 200 MB)".to_string()); }
        let bytes = fs::read(&source).map_err(|_| "Não foi possível ler o arquivo")?;
        let sha = hash(&bytes);
        let base = format!("{sha}.{}", extensao(&name));
        fs::create_dir_all(&dir).map_err(|_| "Não foi possível guardar a cópia do original")?;
        if !dir.join(&base).exists() { fs::write(dir.join(&base), &bytes).map_err(|_| "Não foi possível guardar a cópia do original")?; }
        Ok(json!({"tipo": kind, "nome": name, "base": base, "atual": sha}))
    }).await.map_err(|_| "Falha ao abrir o arquivo")?
}

fn base_valida(base: &str) -> bool {
    let Some((sha, ext)) = base.split_once('.') else { return false };
    sha.len() == 64 && sha.chars().all(|c| c.is_ascii_hexdigit()) && matches!(ext, "pdf" | "png" | "jpg" | "jpeg" | "jfif" | "webp")
}

/// Bytes da cópia intacta (o editor desenha a página a partir dela).
#[tauri::command]
pub fn design_documento_base(state: tauri::State<'_, Core>, base: String) -> Result<tauri::ipc::Response, String> {
    if !base_valida(&base) { return Err("Cópia inválida".into()); }
    fs::read(bases(&state).join(&base)).map(tauri::ipc::Response::new).map_err(|_| "A cópia do original sumiu; abra o arquivo de novo".into())
}

/// Hash do arquivo agora (para ver se mudou fora do editor).
#[tauri::command]
pub async fn design_documento_atual(state: tauri::State<'_, Core>, file: Option<String>, root: Option<String>, path: Option<String>) -> Result<String, String> {
    let (source, _, _) = iris_source(&state, &file, &root, &path, "Arquivos de pastas protegidas não abrem nos designs")?;
    tauri::async_runtime::spawn_blocking(move || fs::read(&source).map(|b| hash(&b)).map_err(|_| "Arquivo não encontrado".to_string()))
        .await.map_err(|_| "Falha ao ler o arquivo")?
}

#[derive(Deserialize)]
pub struct CamadaPagina { pagina: usize, png: String }

/// Grava no original: o PDF base com as camadas carimbadas, ou a imagem já composta pela
/// interface. Só grava se o arquivo ainda for o `esperado` (o último que o editor viu); antes,
/// guarda uma cópia do arquivo atual nos backups da Ayla. Devolve o hash novo.
#[tauri::command]
pub async fn design_documento_gravar(state: tauri::State<'_, Core>, file: Option<String>, root: Option<String>, path: Option<String>, base: String,
    esperado: String, camadas: Option<Vec<CamadaPagina>>, imagem: Option<String>) -> Result<Value, String> {
    if !base_valida(&base) { return Err("Cópia inválida".into()); }
    let (source, name, _) = iris_source(&state, &file, &root, &path, "Arquivos de pastas protegidas não são alterados pelos designs")?;
    let kind = tipo(&name).ok_or("Formato não editável")?;
    let (base_path, backups) = (bases(&state).join(&base), state.root.join("backups").join("designs"));
    let decode = |text: &str| base64::engine::general_purpose::STANDARD.decode(text.as_bytes()).map_err(|_| "Dados da edição inválidos".to_string());
    let camadas: Vec<(usize, Vec<u8>)> = camadas.unwrap_or_default().into_iter().map(|c| Ok((c.pagina, decode(&c.png)?))).collect::<Result<_, String>>()?;
    let imagem = imagem.as_deref().map(decode).transpose()?;
    tauri::async_runtime::spawn_blocking(move || {
        let atual = fs::read(&source).map_err(|_| "Arquivo não encontrado")?;
        if hash(&atual) != esperado {
            return Err("O arquivo mudou fora do editor desde que foi aberto. Feche e abra de novo para editar a versão nova (nada foi gravado).".to_string());
        }
        let novo = if kind == "pdf" {
            let base = fs::read(&base_path).map_err(|_| "A cópia do original sumiu; abra o arquivo de novo")?;
            let camadas = camadas.into_iter().map(|(p, png)| Ok((p, super::super::design_doc::decodificar_png(&png)?))).collect::<Result<Vec<_>, String>>()?;
            super::super::design_doc::carimbar(&base, camadas)?
        } else {
            let bytes = imagem.ok_or("Imagem da edição ausente")?;
            if !super::super::design_doc::mesmo_formato(&extensao(&name), &bytes) { return Err("A imagem editada não está no formato do arquivo".into()); }
            bytes
        };
        fs::create_dir_all(&backups).map_err(|_| "Não foi possível guardar o backup")?;
        let stamp = store::now();
        let copia = backups.join(format!("{stamp}-{name}"));
        fs::write(&copia, &atual).map_err(|_| "Não foi possível guardar o backup")?;
        let tmp = source.with_extension(format!("{}.ayla-tmp", extensao(&name)));
        fs::write(&tmp, &novo).map_err(|_| "Não foi possível gravar o arquivo")?;
        fs::rename(&tmp, &source).map_err(|_| { let _ = fs::remove_file(&tmp); "Não foi possível gravar o arquivo".to_string() })?;
        Ok(json!({"atual": hash(&novo), "backup": copia}))
    }).await.map_err(|_| "A gravação parou")?
}
