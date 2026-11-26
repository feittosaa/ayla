use super::*;
use crate::core::{bases, store::ProjectSource};

fn source(kind: &str, root: &str, path: &str, name: &str) -> ProjectSource {
    ProjectSource { id: String::new(), kind: kind.into(), root: root.into(), path: path.into(), name: name.into() }
}

fn saved(state: &Core, project: &str) -> Result<Vec<ProjectSource>, String> {
    let db = state.store.lock().map_err(|_| "Dados ocupados")?;
    db.project(project).map_err(|_| "Falha ao ler o projeto")?.ok_or("Projeto não encontrado")?;
    db.project_sources(project).map_err(|_| "Falha ao ler os vínculos".into())
}

#[tauri::command]
pub fn project_links(state: tauri::State<'_, Core>, project: String) -> Result<Vec<ProjectSource>, String> {
    saved(&state, &project)
}

async fn options(state: &Core, project: &str, group: &str) -> Result<Vec<ProjectSource>, String> {
    let mut result = Vec::new();
    let own = project_root_id(project);
    let visible = |root: &str| !root.starts_with("project:") || root == own;
    match group {
        "athena" => {
            let roots = state.athena_roots(&state.prefs()?);
            return tauri::async_runtime::spawn_blocking(move || {
                let mut out = Vec::new();
                for root in roots.into_iter().filter(|r| r.available && (r.kind != "project" || r.id == own)) {
                    out.push(source("vault", &root.id, "", &root.name));
                    for note in athena::list(&root.path).into_iter().chain(athena::texts(&root.path)) {
                        if athena_lock::protected_file_of(&root.path,&note.path).is_some() { continue; }
                        out.push(source("note", &root.id, &note.path, &format!("{} / {}", root.name, note.path)));
                    }
                }
                out
            }).await.map_err(|_| "Falha ao listar a Athena".into());
        }
        "iris" => {
            let lib = iris_library_for(state).await?;
            for root in ["roots","athenaRoots"].iter().flat_map(|k| lib[k].as_array().into_iter().flatten()) {
                let id = root["id"].as_str().unwrap_or_default();
                if !visible(id) || root["available"] != true { continue; }
                let name = root["name"].as_str().unwrap_or_default();
                result.push(source("folder", id, "", name));
                for path in root["folders"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                    result.push(source("folder", id, path, &format!("{name} / {path}")));
                }
            }
            for item in lib["items"].as_array().into_iter().flatten() {
                let root = item["root"].as_str().unwrap_or_default();
                if item["kind"] != "image" || !visible(root) { continue; }
                let key = item["key"].as_str().unwrap_or_default();
                let name = item["name"].as_str().unwrap_or_default();
                result.push(source("image", root, key, name));
            }
        }
        "catalog" => {
            for base in bases::catalog(&state.root) {
                if bases::installed(&state.root, &base.id).is_some() {
                    result.push(source("catalog", &base.id, "", &base.name));
                }
            }
        }
        _ => return Err("Origem desconhecida".into()),
    }
    Ok(result)
}

#[tauri::command]
pub async fn project_source_options(state: tauri::State<'_, Core>, project: String, group: String) -> Result<Vec<ProjectSource>, String> {
    saved(&state, &project)?;
    options(&state, &project, &group).await
}

fn group(kind: &str) -> &str {
    match kind { "folder" | "image" => "iris", "vault" | "note" => "athena", _ => "catalog" }
}

#[tauri::command]
pub async fn project_link(state: tauri::State<'_, Core>, project: String, item: ProjectSource) -> Result<(), String> {
    let mut item = options(&state, &project, group(&item.kind)).await?.into_iter()
        .find(|s| s.kind == item.kind && s.root == item.root && s.path == item.path).ok_or("Origem indisponível")?;
    item.id = store::id();
    state.store.lock().map_err(|_| "Dados ocupados")?.link_source(&project, &item).map_err(|_| "Não foi possível conectar a origem".into())
}

pub(super) fn database(root: &std::path::Path, id: &str) -> PathBuf {
    root.join("files").join("project-databases").join(format!("{id}.sqlite"))
}

fn import_database(path: &std::path::Path, dest: &std::path::Path) -> Result<(), String> {
    use rusqlite::{Connection, OpenFlags};
    let name = path.file_name().and_then(|s| s.to_str()).ok_or("Nome de arquivo inválido")?;
    if !fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() <= bases::link::MAX_ARCHIVE) {
        return Err("Escolha um arquivo de até 300 MB".into());
    }
    fs::create_dir_all(dest.parent().ok_or("Pasta inválida")?).map_err(|_| "Falha ao preparar a importação")?;
    match path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "csv" => {
            let bytes = fs::read(path).map_err(|_| "Falha ao ler CSV")?;
            let db = Connection::open(dest).map_err(|_| "Falha ao criar a base")?;
            bases::csv::import(&db, &[(name.to_owned(), bytes)])?;
        }
        "db" | "sqlite" | "sqlite3" => {
            let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|_| "Falha ao abrir SQLite")?;
            db.backup("main", dest, rusqlite::backup::NO_PROGRESS).map_err(|_| "O arquivo não é uma base SQLite legível")?;
        }
        _ => return Err("Escolha uma base CSV ou SQLite".into()),
    }
    Ok(())
}

#[tauri::command]
pub async fn project_import_database(app: tauri::AppHandle, state: tauri::State<'_, Core>, project: String) -> Result<(), String> {
    saved(&state, &project)?;
    let handle = app.clone();
    let file = crate::desktop::panel_dialog(&handle, move || app.dialog().file().add_filter("CSV ou SQLite", &["csv","db","sqlite","sqlite3"])
        .blocking_pick_file().and_then(|p| p.into_path().ok())).await?;
    let Some(path) = file else { return Ok(()) };
    let id = store::id();
    let mut item = source("database", "", &id, &path.file_name().unwrap_or_default().to_string_lossy());
    item.id = id.clone();
    let dest = database(&state.root, &id);
    let output = dest.clone();
    let result = tauri::async_runtime::spawn_blocking(move || import_database(&path, &output)).await.map_err(|_| "Falha na importação")?
        .and_then(|_| state.store.lock().map_err(|_| "Dados ocupados".to_string())?.link_source(&project, &item).map_err(|_| "Falha ao guardar a base".to_string()));
    if result.is_err() { let _ = fs::remove_file(dest); }
    result
}

#[tauri::command]
pub fn project_unlink(state: tauri::State<'_, Core>, project: String, id: String) -> Result<(), String> {
    let item = saved(&state,&project)?.into_iter().find(|s| s.id == id).ok_or("Vínculo não encontrado")?;
    state.store.lock().map_err(|_| "Dados ocupados")?.unlink_source(&project,&id).map_err(|_| "Falha ao desconectar")?;
    if item.kind == "database" { let _ = fs::remove_file(database(&state.root,&id)); }
    Ok(())
}

pub(super) fn selected(state: &Core, conversation: &str, bases: &[String]) -> Result<Vec<ProjectSource>, String> {
    let ids: Vec<_> = bases.iter().filter_map(|s| s.strip_prefix("project-source:")).collect();
    if ids.is_empty() { return Ok(Vec::new()); }
    let project = state.store.lock().map_err(|_| "Dados ocupados")?.conversation_project(conversation)
        .map_err(|_| "Conversa indisponível")?.ok_or("Esta conversa não pertence a um projeto")?;
    let list = saved(state,&project)?;
    ids.into_iter().map(|id| list.iter().find(|s| s.id == id).cloned().ok_or_else(|| "Origem desconectada deste projeto".into())).collect()
}

pub(super) async fn context(state: &Core, prefs: &Preferences, sources: &[ProjectSource], message: &str, token: &CancellationToken,
    texts: &mut Vec<(String,Kind,String)>, images: &mut Vec<PathBuf>) -> Result<(), String> {
    let iris = if sources.iter().any(|s| group(&s.kind) == "iris") { iris_library_for(state).await? } else { Value::Null };
    for s in sources {
        let mut text = String::new();
        match s.kind.as_str() {
            "vault" | "note" => {
                let dir = state.athena_path(prefs,&s.root)?;
                let mut notes = athena::list(&dir);
                notes.extend(athena::texts(&dir));
                notes.retain(|n| athena_lock::protected_file_of(&dir,&n.path).is_none());
                if s.kind == "note" { notes.retain(|n| n.path == s.path); }
                if notes.is_empty() { return Err(format!("Nenhuma nota disponível em {}", s.name)); }
                notes.sort_by_key(|n| std::cmp::Reverse(message.split_whitespace().filter(|w| w.len()>2 && n.path.to_lowercase().contains(&w.to_lowercase())).count()));
                for n in notes.iter().take(8) {
                    let (body,_) = athena::read(&dir,&n.path)?;
                    text.push_str(&format!("\n{}\n{}\n",n.path,body.chars().take(16000).collect::<String>()));
                }
            }
            "folder" | "image" => {
                let mut found = false;
                for item in iris["items"].as_array().into_iter().flatten().filter(|i| i["kind"] == "image") {
                    let path = item["path"].as_str().unwrap_or_default();
                    let matches = if s.kind == "image" { item["key"] == s.path } else {
                        item["root"] == s.root && (s.path.is_empty() || path.starts_with(&format!("{}/",s.path)))
                    };
                    if !matches { continue; }
                    found = true;
                    let name = item["name"].as_str().unwrap_or_default();
                    text.push_str(name); text.push('\n');
                    if images.len() < files::MAX_IMAGES {
                        let field = |k: &str| item[k].as_str().map(str::to_owned);
                        let (path,_,_) = iris_source(state,&field("file"),&field("root"),&field("path"),"Imagem protegida")?;
                        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
                        let image = if matches!(ext.as_str(),"png"|"jpg"|"jpeg") && fs::metadata(&path).is_ok_and(|m| m.len() <= 16 << 20) {
                            path
                        } else {
                            media::thumbnail(&thumbnails(state), &path, false).ok_or("Não foi possível ler a imagem da Iris")?
                        };
                        images.push(image);
                    }
                    if text.len() > 16000 { break; }
                }
                if !found { return Err(format!("Nenhuma imagem disponível em {}",s.name)); }
            }
            "database" | "catalog" => {
                let entry = if s.kind == "catalog" { Some(bases::find(&state.root,&s.root).ok_or("Base fora do catálogo")?) } else { None };
                let path = if entry.is_some() { bases::db_path(&state.root,&s.root) } else { database(&state.root,&s.id) };
                let (sql,table) = base_lookup_at(state,prefs,entry.as_ref(),&s.name,path,message,token).await?;
                text = format!("Consulta: {sql}\n{}",json!({"colunas":table.columns,"linhas":table.rows,"mais":table.more}));
            }
            _ => return Err("Vínculo desconhecido".into()),
        }
        texts.push((s.name.clone(),Kind::Text,text));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn linked_note_reads_current_original_only_for_its_project() {
        let temp = std::env::temp_dir().join(format!("ayla-linked-{}",store::id()));
        let state = Core::open(temp.clone()).unwrap();
        let (project,chat,other) = {
            let db = state.store.lock().unwrap();
            let one = db.create_project("Pesquisa","").unwrap();
            let two = db.create_project("Outro","").unwrap();
            let a = db.create_conversation("A", Some(&one.id)).unwrap();
            let b = db.create_conversation("B", Some(&two.id)).unwrap();
            (one.id,a.id,b.id)
        };
        let dir = state.athena_path(&state.prefs().unwrap(),"ayla").unwrap();
        fs::write(dir.join("Nota.md"),"Versão inicial").unwrap();
        let mut note = options(&state,&project,"athena").await.unwrap().into_iter().find(|s| s.path=="Nota.md").unwrap();
        note.id = store::id();
        state.store.lock().unwrap().link_source(&project,&note).unwrap();
        let choice = [format!("project-source:{}",note.id)];
        assert!(selected(&state,&other,&choice).is_err());
        assert!(selected(&state,&chat,&[]).unwrap().is_empty());
        let chosen = selected(&state,&chat,&choice).unwrap();
        fs::write(dir.join("Nota.md"),"Versão atual").unwrap();
        let mut text = Vec::new();
        context(&state,&state.prefs().unwrap(),&chosen,"nota",&CancellationToken::new(),&mut text,&mut Vec::new()).await.unwrap();
        assert!(text[0].2.contains("Versão atual"));
        fs::remove_file(dir.join("Nota.md")).unwrap();
        assert!(context(&state,&state.prefs().unwrap(),&chosen,"nota",&CancellationToken::new(),&mut text,&mut Vec::new()).await.is_err());
        state.store.lock().unwrap().unlink_source(&project,&note.id).unwrap();
        assert!(selected(&state,&chat,&choice).is_err());
        drop(state);
        fs::remove_dir_all(temp).unwrap();
    }

    #[tokio::test]
    async fn iris_folder_and_image_lose_access_after_disconnect() {
        let temp = std::env::temp_dir().join(format!("ayla-linked-iris-{}",store::id()));
        let state = Core::open(temp.join("ayla")).unwrap();
        let dir = temp.join("Fotos");
        fs::create_dir_all(dir.join("Mapas")).unwrap();
        fs::write(dir.join("Mapas/rota.png"), b"fixture").unwrap();
        let mut prefs = state.prefs().unwrap();
        prefs.iris_folders = vec![dir.clone()];
        state.preferences.lock().unwrap().save(&prefs).unwrap();
        let options = options(&state,"example","iris").await.unwrap();
        for selected in [("folder","Mapas"),("image","rota.png")] {
            let item = options.iter().find(|s| s.kind == selected.0 && s.name.ends_with(selected.1)).unwrap();
            let mut images = Vec::new();
            context(&state,&prefs,std::slice::from_ref(item),"mapa",&CancellationToken::new(),&mut Vec::new(),&mut images).await.unwrap();
            assert_eq!(images, [dir.join("Mapas/rota.png")]);
        }
        prefs.iris_folders.clear();
        state.preferences.lock().unwrap().save(&prefs).unwrap();
        let image = options.iter().find(|s| s.kind == "image").unwrap();
        assert!(context(&state,&prefs,std::slice::from_ref(image),"mapa",&CancellationToken::new(),&mut Vec::new(),&mut Vec::new()).await.is_err());
        drop(state);
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn csv_and_sqlite_are_independent_queryable_copies() {
        let dir = std::env::temp_dir().join(format!("ayla-local-db-{}",store::id()));
        fs::create_dir_all(&dir).unwrap();
        let csv = dir.join("amostras.csv");
        fs::write(&csv,"cor;quantidade\nazul;3\n").unwrap();
        let db = dir.join("importada.sqlite");
        import_database(&csv,&db).unwrap();
        let copy = dir.join("segunda.sqlite");
        import_database(&db,&copy).unwrap();
        fs::remove_file(csv).unwrap();
        fs::remove_file(db).unwrap();
        assert_eq!(bases::query(&copy,"SELECT COR, QUANTIDADE FROM AMOSTRAS").unwrap().rows[0], ["azul","3"]);
        fs::remove_dir_all(dir).unwrap();
    }
}
