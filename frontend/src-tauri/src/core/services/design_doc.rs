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

/// Folha do PDF montado: `origem` (página do original, a partir de 0) ou uma página em branco nova
/// com `largura` e `altura` em pontos.
#[derive(Deserialize)]
pub struct FolhaPdf { origem: Option<usize>, largura: Option<f64>, altura: Option<f64> }

fn plano(lista: Option<Vec<FolhaPdf>>) -> Result<Option<Vec<super::super::design_doc::Folha>>, String> {
    use super::super::design_doc::Folha;
    lista.map(|l| l.into_iter().map(|f| match (f.origem, f.largura, f.altura) {
        (Some(i), _, _) => Ok(Folha::Original(i)),
        (None, Some(w), Some(h)) => Ok(Folha::Branca(w, h)),
        _ => Err("Página inválida".to_string()),
    }).collect()).transpose()
}

/// Página a mais de uma imagem editada (a imagem não comporta várias): vira outro arquivo na mesma
/// pasta. `arquivo`/`hash`: o arquivo que esta página gravou da última vez (regravado só se ainda
/// estiver como ficou).
#[derive(Deserialize)]
pub struct PaginaExtra { imagem: String, arquivo: Option<String>, hash: Option<String> }

fn decodificar(text: &str) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::STANDARD.decode(text.as_bytes()).map_err(|_| "Dados da edição inválidos".to_string())
}

/// Grava no original: o PDF base com as camadas carimbadas (e, com `folhas`, as páginas na nova
/// ordem, com as novas em branco), ou a imagem já composta pela interface (as páginas a mais viram
/// outras imagens na mesma pasta: "Nome 2.png", "Nome 3.png"…). Só grava se o arquivo ainda for o
/// `esperado` (o último que o editor viu); antes, guarda uma cópia do arquivo atual nos backups da
/// Ayla. Devolve o hash novo e onde ficou cada página a mais.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn design_documento_gravar(state: tauri::State<'_, Core>, file: Option<String>, root: Option<String>, path: Option<String>, base: String,
    esperado: String, camadas: Option<Vec<CamadaPagina>>, imagem: Option<String>, folhas: Option<Vec<FolhaPdf>>, extras: Option<Vec<PaginaExtra>>) -> Result<Value, String> {
    if !base_valida(&base) { return Err("Cópia inválida".into()); }
    let (source, name, _) = iris_source(&state, &file, &root, &path, "Arquivos de pastas protegidas não são alterados pelos designs")?;
    let kind = tipo(&name).ok_or("Formato não editável")?;
    let (base_path, backups) = (bases(&state).join(&base), state.root.join("backups").join("designs"));
    let camadas: Vec<(usize, Vec<u8>)> = camadas.unwrap_or_default().into_iter().map(|c| Ok((c.pagina, decodificar(&c.png)?))).collect::<Result<_, String>>()?;
    let imagem = imagem.as_deref().map(decodificar).transpose()?;
    let folhas = plano(folhas)?;
    let extras: Vec<(Vec<u8>, Option<String>, Option<String>)> = extras.unwrap_or_default().into_iter().map(|e| Ok((decodificar(&e.imagem)?, e.arquivo, e.hash))).collect::<Result<_, String>>()?;
    if kind == "imagem" && !extras.iter().all(|(b, _, _)| super::super::design_doc::mesmo_formato(&extensao(&name), b)) {
        return Err("A imagem editada não está no formato do arquivo".into());
    }
    let (fonte, nome, copias) = (source.clone(), name.clone(), backups.clone());
    let atual = tauri::async_runtime::spawn_blocking(move || {
        let source = fonte; let name = nome; let backups = copias;
        let atual = fs::read(&source).map_err(|_| "Arquivo não encontrado")?;
        if hash(&atual) != esperado {
            return Err("O arquivo mudou fora do editor desde que foi aberto. Feche e abra de novo para editar a versão nova (nada foi gravado).".to_string());
        }
        let novo = if kind == "pdf" {
            let base = fs::read(&base_path).map_err(|_| "A cópia do original sumiu; abra o arquivo de novo")?;
            let camadas = camadas.into_iter().map(|(p, png)| Ok((p, super::super::design_doc::decodificar_png(&png)?))).collect::<Result<Vec<_>, String>>()?;
            super::super::design_doc::montar(&base, folhas.as_deref(), camadas)?
        } else {
            let bytes = imagem.ok_or("Imagem da edição ausente")?;
            if !super::super::design_doc::mesmo_formato(&extensao(&name), &bytes) { return Err("A imagem editada não está no formato do arquivo".into()); }
            bytes
        };
        gravar_com_backup(&source, &name, &atual, &novo, &backups)?;
        Ok(hash(&novo))
    }).await.map_err(|_| "A gravação parou")??;
    let mut gravados = Vec::new();
    for (indice, (bytes, arquivo, anterior)) in extras.into_iter().enumerate() {
        gravados.push(gravar_extra(&state, &file, &root, &path, &source, &name, indice + 2, &bytes, arquivo, anterior, &backups)?);
    }
    Ok(json!({"atual": atual, "extras": gravados}))
}

/// Grava por um temporário, depois de guardar uma cópia do que havia nos backups.
fn gravar_com_backup(source: &std::path::Path, name: &str, atual: &[u8], novo: &[u8], backups: &std::path::Path) -> Result<(), String> {
    fs::create_dir_all(backups).map_err(|_| "Não foi possível guardar o backup")?;
    let copia = backups.join(format!("{}-{name}", store::now()));
    fs::write(&copia, atual).map_err(|_| "Não foi possível guardar o backup")?;
    let tmp = source.with_extension(format!("{}.ayla-tmp", extensao(name)));
    fs::write(&tmp, novo).map_err(|_| "Não foi possível gravar o arquivo")?;
    fs::rename(&tmp, source).map_err(|_| { let _ = fs::remove_file(&tmp); "Não foi possível gravar o arquivo".to_string() })
}

/// Uma página a mais da imagem: regrava o arquivo dela se ainda estiver como a Ayla deixou;
/// senão (ou na primeira vez), cria "Nome N.ext" ao lado, sem sobrescrever nada. Arquivos
/// guardados na Ayla (Iris › Arquivos) viram outro arquivo guardado.
#[allow(clippy::too_many_arguments)]
fn gravar_extra(state: &Core, file: &Option<String>, root: &Option<String>, path: &Option<String>, source: &std::path::Path, name: &str, numero: usize,
    bytes: &[u8], arquivo: Option<String>, anterior: Option<String>, backups: &std::path::Path) -> Result<Value, String> {
    let ext = extensao(name);
    let stem = name.strip_suffix(&format!(".{}", name.rsplit('.').next().unwrap_or(""))).unwrap_or(name);
    let sugerido = format!("{stem} {numero}.{ext}");
    if file.is_some() {
        // Guardado na Ayla: o id do arquivo da página, se ainda for o mesmo conteúdo.
        if let (Some(id), Some(anterior)) = (arquivo.as_deref().and_then(|a| a.strip_prefix("arquivo:")), anterior.as_deref()) {
            let info = state.store.lock().map_err(|_| "Dados ocupados")?.file(id).ok().flatten();
            if let Some(info) = info {
                let destino = state.root.join(&info.path);
                if let Ok(atual) = fs::read(&destino) {
                    if hash(&atual) == anterior {
                        gravar_com_backup(&destino, &info.name, &atual, bytes, backups)?;
                        let novo = hash(bytes);
                        state.store.lock().map_err(|_| "Dados ocupados")?.replace_file_content(id, bytes.len() as i64, &novo).map_err(|_| "Não foi possível registrar a página")?;
                        return Ok(json!({"arquivo": format!("arquivo:{id}"), "hash": novo, "nome": info.name}));
                    }
                }
            }
        }
        let saved = state.store_file(&sugerido, |dest| fs::write(dest, bytes).map(|_| bytes.len() as u64).map_err(|_| "Não foi possível guardar a página".to_string()), None, true)?;
        return Ok(json!({"arquivo": format!("arquivo:{}", saved.id), "hash": hash(bytes), "nome": saved.name}));
    }
    let (Some(_), Some(relativo)) = (root, path) else { return Err("Arquivo não informado".into()) };
    let pasta = source.parent().ok_or("Pasta do arquivo não encontrada")?;
    let pasta_rel = relativo.rsplit_once('/').map_or("", |(p, _)| p);
    let junta = |nome: &str| if pasta_rel.is_empty() { nome.to_string() } else { format!("{pasta_rel}/{nome}") };
    // O arquivo anterior só vale se for um nome simples na mesma pasta do original.
    let anterior_nome = arquivo.as_deref().and_then(|a| a.strip_prefix(&junta(""))).filter(|n| !n.is_empty() && !n.contains('/') && !n.starts_with('.') && extensao(n) == ext);
    if let (Some(nome), Some(anterior)) = (anterior_nome, anterior.as_deref()) {
        let destino = pasta.join(nome);
        if let Ok(atual) = fs::read(&destino) {
            if hash(&atual) == anterior {
                gravar_com_backup(&destino, nome, &atual, bytes, backups)?;
                return Ok(json!({"arquivo": junta(nome), "hash": hash(bytes), "nome": nome}));
            }
        }
    }
    let destino = super::free_path(pasta, &sugerido);
    let nome = destino.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or("Nome inválido")?;
    let tmp = pasta.join(format!(".{nome}.ayla-tmp"));
    fs::write(&tmp, bytes).map_err(|_| "Não foi possível gravar a página")?;
    fs::rename(&tmp, &destino).map_err(|_| { let _ = fs::remove_file(&tmp); "Não foi possível gravar a página".to_string() })?;
    Ok(json!({"arquivo": junta(&nome), "hash": hash(bytes), "nome": nome}))
}

/// Exporta o PDF editado (carimbado a partir da base, com as páginas na ordem do editor) para a
/// Iris ou o computador, sem mexer no original: o texto do PDF continua selecionável.
#[tauri::command]
pub async fn design_documento_exportar(app: tauri::AppHandle, state: tauri::State<'_, Core>, name: String, base: String,
    camadas: Option<Vec<CamadaPagina>>, folhas: Option<Vec<FolhaPdf>>, target: String) -> Result<Value, String> {
    if !base_valida(&base) || !base.ends_with(".pdf") { return Err("Cópia inválida".into()); }
    let base_path = bases(&state).join(&base);
    let camadas: Vec<(usize, Vec<u8>)> = camadas.unwrap_or_default().into_iter().map(|c| Ok((c.pagina, decodificar(&c.png)?))).collect::<Result<_, String>>()?;
    let folhas = plano(folhas)?;
    let pdf = tauri::async_runtime::spawn_blocking(move || {
        let base = fs::read(&base_path).map_err(|_| "A cópia do original sumiu; abra o arquivo de novo")?;
        let camadas = camadas.into_iter().map(|(p, png)| Ok((p, super::super::design_doc::decodificar_png(&png)?))).collect::<Result<Vec<_>, String>>()?;
        super::super::design_doc::montar(&base, folhas.as_deref(), camadas)
    }).await.map_err(|_| "A exportação parou")??;
    super::deliver_export(&app, &state, vec![(super::super::design::export_name(&name, "pdf"), pdf)], &target).await
}
