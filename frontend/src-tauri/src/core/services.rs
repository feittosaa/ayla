use super::{
    cloud::{self, Api, Login, Secret},
    files::{self, Kind},
    athena, athena_git, athena_lock,
    hardware,
    iris,
    media,
    reminders,
    memory::{self, Fact, Privacy},
    models::{self, Capability, Model},
    notify,
    policy::{self, Sensitivity, UsageMode},
    preferences::{PreferenceStore, Preferences},
    runtime::{self, Runtime},
    store::{self, FileInfo, Scope, Store},
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::ipc::Channel;
use tauri_plugin_dialog::DialogExt;
use tokio_util::sync::CancellationToken;

pub struct Core {
    pub root: PathBuf,
    preferences: Mutex<PreferenceStore>,
    store: Mutex<Store>,
    pub runtime: Arc<Runtime>,
    install_gate: tokio::sync::Mutex<()>,
    /// Renovação do acesso ao ChatGPT: uma por vez (o token de renovação é rotativo).
    cloud_gate: tokio::sync::Mutex<()>,
    jobs: Mutex<HashMap<String, CancellationToken>>,
    keys: Mutex<HashMap<String, String>>,
    /// Última atividade da conversa; a extração automática espera a pausa.
    last_activity: Mutex<Instant>,
    /// Extração em segundo plano, cancelada quando o usuário volta a conversar.
    background: Mutex<Option<CancellationToken>>,
    /// Estado do preparo automático, exibido pela interface.
    setup: Mutex<Value>,
    /// Voz e transcrição em fila própria: rodam enquanto a conversa gera a resposta.
    voice_gate: tokio::sync::Mutex<()>,
}
impl Core {
    pub fn open(root: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&root).map_err(|_| "Não foi possível abrir seus dados")?;
        // Pasta de imagens coladas de versões anteriores (agora viram arquivos guardados).
        let _ = fs::remove_dir_all(root.join("work").join("pasted"));
        let store = Store::open(&root.join("ayla.db"))
            .map_err(|_| "Não foi possível abrir o banco da Ayla")?;
        clean_files(&root, &store);
        Ok(Self {
            preferences: Mutex::new(PreferenceStore::new(&root)),
            store: Mutex::new(store),
            runtime: Arc::new(Runtime::new(root.join("runtimes"))),
            root,
            install_gate: tokio::sync::Mutex::new(()),
            cloud_gate: tokio::sync::Mutex::new(()),
            jobs: Mutex::new(HashMap::new()),
            keys: Mutex::new(HashMap::new()),
            last_activity: Mutex::new(Instant::now()),
            background: Mutex::new(None),
            setup: Mutex::new(json!({"active":false})),
            voice_gate: tokio::sync::Mutex::new(()),
        })
    }
    pub(crate) fn prefs(&self) -> Result<Preferences, String> {
        self.preferences
            .lock()
            .map_err(|_| "Configuração ocupada")?
            .load()
            .map_err(|_| "Configuração inválida; arquivo preservado".into())
    }
    pub(super) fn budget(&self, prefs: &Preferences) -> u64 {
        let mut hw = hardware::detect();
        hw.ram_available_bytes = hw
            .ram_available_bytes
            .saturating_add(self.runtime.resident_bytes())
            .min(hw.ram_total_bytes);
        models::memory_budget(&hw, prefs)
    }
    pub(super) fn models(&self) -> Result<Vec<Model>, String> {
        self.store
            .lock()
            .map_err(|_| "Dados ocupados")?
            .models()
            .map_err(|_| "Não foi possível consultar modelos")?
            .into_iter()
            .map(|raw| serde_json::from_str(&raw).map_err(|_| "Registro de modelo inválido".into()))
            .collect()
    }
    /// A extração em segundo plano cede a vez; outra resposta em andamento é aguardada.
    pub(crate) async fn take_gate(&self, token: &CancellationToken, waiting: impl FnOnce()) -> Result<tokio::sync::MutexGuard<'_, ()>, String> {
        let background = self.background.lock().ok().and_then(|mut b| b.take());
        if let Ok(gate) = self.runtime.gate.try_lock() { return Ok(gate); }
        match background { Some(task) => task.cancel(), None => waiting() }
        tokio::select! {
            _ = token.cancelled() => Err("Resposta interrompida".into()),
            gate = self.runtime.gate.lock() => Ok(gate),
        }
    }
    fn job(&self, id: &str) -> Result<CancellationToken, String> {
        if id.is_empty() || id.len() > 100 {
            return Err("Identificador de tarefa inválido".into());
        }
        let mut jobs = self.jobs.lock().map_err(|_| "Tarefas ocupadas")?;
        if jobs.contains_key(id) {
            return Err("Essa tarefa já está em execução".into());
        }
        let token = CancellationToken::new();
        jobs.insert(id.into(), token.clone());
        Ok(token)
    }
    fn finish(&self, id: &str) {
        if let Ok(mut jobs) = self.jobs.lock() {
            jobs.remove(id);
        }
    }
    pub fn shutdown(&self) {
        if let Ok(jobs) = self.jobs.lock() {
            for token in jobs.values() {
                token.cancel();
            }
        }
        self.runtime.stop_all();
    }
    fn key(&self) -> Result<String, String> {
        let endpoint = self.prefs()?.cloud_endpoint;
        if let Some(key) = self
            .keys
            .lock()
            .map_err(|_| "Credencial ocupada")?
            .get(&endpoint)
        {
            return Ok(key.clone());
        }
        keyring::Entry::new("dev.feit.ayla", &endpoint)
            .and_then(|entry| entry.get_password())
            .map_err(|_| "Configure a chave do provedor cloud antes de usar".into())
    }
}
impl Core {
    fn memory_model(&self, prefs: &Preferences) -> Option<Model> {
        let models = self.models().ok()?;
        models::choose(&models, Capability::Embedding, self.budget(prefs), prefs.context_tokens, None).ok()
    }
    /// Calcula vetores ausentes (memórias novas, editadas ou migradas) em lotes pequenos.
    async fn backfill(&self, prefs: &Preferences, model: &Model, cancel: &CancellationToken) -> Result<(), String> {
        loop {
            let pending = self.store.lock().map_err(|_| "Dados ocupados")?.facts_without_vector(&model.id, 32).map_err(|_| "Falha ao consultar memórias")?;
            if pending.is_empty() {
                return Ok(());
            }
            let texts: Vec<String> = pending.iter().map(|f| memory::document_text(&f.content)).collect();
            let vectors = self.runtime.embed_many(model, prefs, &texts, cancel).await?;
            let db = self.store.lock().map_err(|_| "Dados ocupados")?;
            for (fact, vector) in pending.iter().zip(vectors) {
                db.set_vector(&fact.id, &model.id, &vector).map_err(|_| "Falha ao salvar memória")?;
            }
        }
    }
    /// Busca híbrida: vetores acima do limiar + FTS5, fundidos por rank.
    fn search(&self, model_id: &str, vector: &[f32], text: &str, minimum: f64, limit: usize, scope: Scope) -> Result<Vec<(Fact, Option<f64>)>, String> {
        let db = self.store.lock().map_err(|_| "Dados ocupados")?;
        let mut hits: Vec<(String, f64)> = db.vectors(model_id, scope).map_err(|_| "Falha ao consultar memórias")?
            .into_iter()
            .filter_map(|(id, v)| memory::cosine(vector, &v).filter(|s| *s >= minimum).map(|s| (id, s)))
            .collect();
        hits.sort_by(|a, b| b.1.total_cmp(&a.1));
        hits.truncate(20);
        let words = memory::fts_query(text).map(|q| db.search_text(&q, 20, scope).unwrap_or_default()).unwrap_or_default();
        let ids = memory::fuse(&hits, &words, limit);
        let facts = db.facts_by_ids(&ids).map_err(|_| "Falha ao consultar memórias")?;
        Ok(facts.into_iter().map(|f| { let score = hits.iter().find(|(i, _)| *i == f.id).map(|(_, s)| *s); (f, score) }).collect())
    }
    /// Memórias relevantes ao pedido, para respostas locais. Falhas não impedem a conversa.
    /// Num projeto, entram as memórias dele e as gerais; nunca as de outro projeto.
    async fn recall(&self, prefs: &Preferences, text: &str, project: Option<&str>, cancel: &CancellationToken) -> Vec<Fact> {
        let Some(model) = self.memory_model(prefs) else { return Vec::new() };
        let result = async {
            self.backfill(prefs, &model, cancel).await?;
            let vector = self.runtime.embed(&model, prefs, &memory::query_text(text), cancel).await?;
            self.search(&model.id, &vector, text, memory::RETRIEVAL_MIN, memory::CONTEXT_LIMIT, Scope::visible(project))
        }
        .await;
        result.map(|items| items.into_iter().map(|(f, _)| f).collect()).unwrap_or_default()
    }
    /// Raízes da Athena: as notas da própria Athena, os cofres (ex.: do Obsidian) e os projetos da Ayla,
    /// cada projeto com a própria pasta de notas.
    fn athena_roots(&self, prefs: &Preferences) -> Vec<AthenaRoot> {
        let mut out = vec![AthenaRoot { id: "ayla".into(), kind: "ayla", name: "Athena".into(), path: self.root.join("athena"), available: true }];
        for vault in prefs.vaults() {
            let path = PathBuf::from(&vault);
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| vault.clone());
            out.push(AthenaRoot { id: format!("vault:{vault}"), kind: "vault", available: path.is_dir(), name, path });
        }
        if let Ok(projects) = self.store.lock().map(|db| db.projects().unwrap_or_default()) {
            for project in projects {
                out.push(AthenaRoot { id: project_root_id(&project.id), kind: "project", name: project.name, path: self.root.join("projetos-notas").join(&project.id), available: true });
            }
        }
        out
    }
    /// Iris: imagens, vídeos e PDFs guardados na Ayla, com a origem (anexo enviado no chat,
    /// imagem gerada pelo chat ou adicionado direto em Arquivos) e a data em que entraram.
    fn iris_stored(&self) -> Result<Vec<(i64, Value)>, String> {
        let stored = self.store.lock().map_err(|_| "Dados ocupados")?.media_files().map_err(|_| "Falha ao consultar os arquivos")?;
        let mut items: Vec<(i64, Value)> = Vec::new();
        for (file, role, conversation, project) in stored {
            let Some(mime) = stored_mime(&file) else { continue };
            let root = project.as_ref().map(|(id, _)| project_root_id(id));
            let project = project.map(|(_, name)| name);
            let (source, place) = match role.as_deref() {
                Some("assistant") => ("gerada", conversation.clone().unwrap_or_else(|| "Conversa".into())),
                Some(_) => ("chat", conversation.clone().unwrap_or_else(|| "Conversa".into())),
                None => ("arquivos", project.clone().unwrap_or_else(|| "Geral".into())),
            };
            let modified = fs::metadata(self.root.join(&file.path)).and_then(|m| m.modified()).ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(file.created_at, |d| d.as_secs() as i64);
            items.push((modified, json!({
                "key": format!("arquivo:{}", file.id), "kind": kind_of_mime(mime), "name": file.name, "source": source, "place": place,
                "project": project, "root": root, "added": file.created_at, "date": modified, "bytes": file.bytes, "file": file.id, "conversation": file.conversation_id,
            })));
        }
        Ok(items)
    }
    /// Pasta de uma raiz, criada se for da Ayla (notas ou projeto); cofres precisam existir.
    fn athena_path(&self, prefs: &Preferences, id: &str) -> Result<PathBuf, String> {
        let root = self.athena_roots(prefs).into_iter().find(|r| r.id == id).ok_or("Esta pasta de notas não está mais na Athena")?;
        if root.kind != "vault" {
            fs::create_dir_all(&root.path).map_err(|_| "Não foi possível criar a pasta de notas")?;
        }
        if !root.path.is_dir() {
            return Err(format!("O cofre {} não está disponível neste computador.", root.name));
        }
        Ok(root.path)
    }
    /// Reindexa as notas que mudaram (pela data e pelo tamanho) e esquece as que saíram.
    /// Devolve a chave da pasta ("/caminho/") usada no índice.
    async fn sync_notes(&self, root: &std::path::Path) -> Result<String, String> {
        let key = format!("{}/", root.to_string_lossy().trim_end_matches('/'));
        let stamps = self.store.lock().map_err(|_| "Dados ocupados")?.note_stamps().map_err(|_| "Falha ao consultar notas")?;
        let (dir, prefix) = (root.to_path_buf(), key.clone());
        let (changed, gone) = tauri::async_runtime::spawn_blocking(move || {
            let notes = athena::list(&dir);
            let mut seen = std::collections::HashSet::new();
            let mut changed = Vec::new();
            for note in notes {
                let full = format!("{prefix}{}", note.path);
                seen.insert(full.clone());
                if stamps.get(&full) == Some(&(note.modified, note.bytes as i64)) {
                    continue;
                }
                let text = athena::read(&dir, &note.path).map(|(t, _)| t).unwrap_or_default();
                let chunks = iris::split(&format!("{}\n\n{text}", note.title));
                changed.push((full, note.modified, note.bytes as i64, chunks));
            }
            let gone: Vec<String> = stamps.keys().filter(|k| k.starts_with(&prefix) && !seen.contains(*k)).cloned().collect();
            (changed, gone)
        }).await.map_err(|_| "Falha ao ler as notas")?;
        let db = self.store.lock().map_err(|_| "Dados ocupados")?;
        for (note, modified, bytes, chunks) in changed {
            db.index_note(&note, modified, bytes, &chunks).map_err(|_| "Falha ao indexar notas")?;
        }
        for note in gone {
            let _ = db.forget_note(&note);
        }
        Ok(key)
    }
    /// Athena na conversa: trechos das notas relevantes ao pedido, como (título, texto). Entram as
    /// notas da Athena, os cofres e, numa conversa de projeto, as notas daquele projeto (as de outros
    /// projetos ficam de fora, como a memória). Uma pasta que corresponde ao pedido fornece
    /// trechos de notas distintas. Para os demais pedidos, vale a busca por significado ou palavras.
    async fn notes_context(&self, prefs: &Preferences, message: &str, project: Option<&str>, cancel: &CancellationToken) -> Vec<(String, String)> {
        let result: Result<Vec<(String, String)>, String> = async {
            let mut roots = Vec::new();
            for root in self.athena_roots(prefs) {
                let visible = match root.kind { "project" => project.is_some_and(|p| root.id == format!("project:{p}")), _ => true };
                if !visible || (root.kind == "vault" && !root.available) {
                    continue;
                }
                if root.kind != "vault" && !root.path.is_dir() {
                    continue;
                }
                if let Ok(key) = self.sync_notes(&root.path).await {
                    roots.push((key, root.name));
                }
            }
            let query = memory::fts_query(message);
            let excerpt = |(note, text): (String, String)| {
                let filename = note.rsplit('/').next().unwrap_or(&note);
                let title = filename.rsplit_once('.').map_or(filename, |(title, _)| title);
                (title.to_string(), text)
            };
            if athena::collection_question(message) {
                let db = self.store.lock().map_err(|_| "Dados ocupados")?;
                let paths = roots.iter().flat_map(|(key, name)| {
                    db.indexed_note_paths(key).unwrap_or_default().into_iter()
                        .map(move |path| (key.clone(), name.clone(), path))
                });
                if let Some(folder) = athena::matching_folder(&memory::search_terms(message), paths) {
                    const LIMIT: usize = 4;
                    let mut ids = query.as_ref()
                        .and_then(|q| db.search_note_chunks(q, &folder, 32).ok())
                        .unwrap_or_default();
                    ids.extend(db.note_openings(&folder, LIMIT).unwrap_or_default());
                    let mut seen = std::collections::HashSet::new();
                    let notes: Vec<_> = db.note_chunks_by_ids(&ids).unwrap_or_default().into_iter()
                        .filter(|(note, _)| seen.insert(note.clone()))
                        .take(LIMIT)
                        .map(excerpt)
                        .collect();
                    if !notes.is_empty() {
                        return Ok(notes);
                    }
                }
            }
            let chosen: Vec<i64> = match self.memory_model(prefs) {
                Some(model) => {
                    for (key, _) in &roots {
                        let pending = self.store.lock().map_err(|_| "Dados ocupados")?.note_chunks_without_vector(&model.id, key, 64).unwrap_or_default();
                        if pending.is_empty() {
                            continue;
                        }
                        let texts: Vec<String> = pending.iter().map(|(_, t)| memory::document_text(t)).collect();
                        if let Ok(vectors) = self.runtime.embed_many(&model, prefs, &texts, cancel).await {
                            let db = self.store.lock().map_err(|_| "Dados ocupados")?;
                            for ((id, _), vector) in pending.iter().zip(vectors) {
                                let _ = db.set_note_vector(*id, &model.id, &vector);
                            }
                        }
                    }
                    let query = self.runtime.embed(&model, prefs, &memory::query_text(message), cancel).await?;
                    let mut hits: Vec<(i64, f64)> = Vec::new();
                    for (key, _) in &roots {
                        hits.extend(self.store.lock().map_err(|_| "Dados ocupados")?.note_vectors(&model.id, key).unwrap_or_default().into_iter()
                            .filter_map(|(id, v)| memory::cosine(&query, &v).filter(|s| *s >= 0.42).map(|s| (id, s))));
                    }
                    hits.sort_by(|a, b| b.1.total_cmp(&a.1));
                    hits.into_iter().take(4).map(|(id, _)| id).collect()
                }
                None => match query {
                    Some(query) => {
                        let db = self.store.lock().map_err(|_| "Dados ocupados")?;
                        roots.iter().flat_map(|(key, _)| db.search_note_chunks(&query, key, 3).unwrap_or_default()).take(3).collect()
                    }
                    None => Vec::new(),
                },
            };
            let rows = self.store.lock().map_err(|_| "Dados ocupados")?.note_chunks_by_ids(&chosen).unwrap_or_default();
            Ok(rows.into_iter().map(excerpt).collect())
        }
        .await;
        result.unwrap_or_default()
    }
    /// Memórias públicas relevantes ao pedido, para a revisão de envio à nuvem (até 5).
    async fn references(&self, prefs: &Preferences, project: Option<&str>, message: &str, cancel: &CancellationToken) -> Vec<Fact> {
        let mut facts = self.recall(prefs, message, project, cancel).await;
        if facts.is_empty() {
            // Sem modelo de memória: busca por palavras.
            if let (Some(query), Ok(db)) = (memory::fts_query(message), self.store.lock()) {
                let ids = db.search_text(&query, 10, Scope::visible(project)).unwrap_or_default();
                facts = db.facts_by_ids(&ids).unwrap_or_default();
            }
        }
        facts.retain(|f| f.privacy == Privacy::Public);
        facts.truncate(memory::MAX_REFERENCES);
        facts
    }
    /// Arquivos que a Iris consulta: os da conversa e, num projeto, os do projeto.
    fn scope_files(&self, conversation: &str, skip: &[String]) -> Result<Vec<FileInfo>, String> {
        let db = self.store.lock().map_err(|_| "Dados ocupados")?;
        let project = db.conversation_project(conversation).map_err(|_| "Conversa não encontrada")?;
        let mut list = db.conversation_files(conversation).unwrap_or_default();
        if let Some(project) = project.as_deref() {
            list.extend(db.project_files(Some(project)).unwrap_or_default());
        }
        let mut seen = std::collections::HashSet::new();
        list.retain(|f| !skip.contains(&f.id) && seen.insert(f.id.clone()));
        Ok(list)
    }
    /// Iris: fotos para comparar no tempo, pela data em que foram tiradas (EXIF; sem ela, a
    /// data em que entraram na Ayla). Do período citado, até `files::MAX_IMAGES` espaçadas.
    async fn iris_photos(&self, conversation: &str, message: &str, skip: &[String]) -> Vec<(FileInfo, i64)> {
        let Ok(files) = self.scope_files(conversation, skip) else { return Vec::new() };
        let root = self.root.clone();
        let dated = tauri::async_runtime::spawn_blocking(move || {
            files.into_iter()
                .filter(|f| f.kind == Kind::Image)
                .map(|f| { let date = iris::photo_date(&root.join(&f.path)).unwrap_or(f.created_at); (f, date) })
                .collect::<Vec<_>>()
        }).await.unwrap_or_default();
        let mut chosen = iris::within_period(&dated, iris::period(message));
        chosen.sort_by_key(|(f, date)| (*date, f.created_at));
        iris::spread(&chosen, files::MAX_IMAGES)
    }
    /// Iris: trechos dos arquivos do projeto e da conversa mais relevantes para o pedido.
    /// Divide e indexa o que ainda não foi indexado; busca por palavras sempre e, havendo modelo
    /// de embeddings, também por significado. Anexos desta mensagem (`skip`) já vão inteiros.
    async fn iris(&self, prefs: &Preferences, conversation: &str, message: &str, skip: &[String], cancel: &CancellationToken) -> Vec<store::Chunk> {
        let result: Result<Vec<store::Chunk>, String> = async {
            let files = self.scope_files(conversation, skip)?;
            for file in &files {
                if self.store.lock().map_err(|_| "Dados ocupados")?.indexed(&file.id).unwrap_or(true) {
                    continue;
                }
                let (kind, path, extracted) = (file.kind, self.root.join(&file.path), file.extracted.clone());
                let chunks = tauri::async_runtime::spawn_blocking(move || iris::file_chunks(kind, &path, extracted.as_deref())).await.unwrap_or_default();
                let _ = self.store.lock().map_err(|_| "Dados ocupados")?.index_file(&file.id, &chunks);
            }
            let ids: Vec<String> = files.iter().map(|f| f.id.clone()).collect();
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            let words: Vec<String> = match memory::fts_query(message) {
                Some(query) => self.store.lock().map_err(|_| "Dados ocupados")?.search_chunks(&query, &ids, 20).unwrap_or_default().into_iter().map(|id| id.to_string()).collect(),
                None => Vec::new(),
            };
            let mut hits: Vec<(String, f64)> = Vec::new();
            if let Some(model) = self.memory_model(prefs) {
                let pending = self.store.lock().map_err(|_| "Dados ocupados")?.chunks_without_vector(&model.id, &ids, iris::EMBED_PER_REQUEST).unwrap_or_default();
                if !pending.is_empty() {
                    let texts: Vec<String> = pending.iter().map(|(_, t)| memory::document_text(t)).collect();
                    if let Ok(vectors) = self.runtime.embed_many(&model, prefs, &texts, cancel).await {
                        let db = self.store.lock().map_err(|_| "Dados ocupados")?;
                        for ((id, _), vector) in pending.iter().zip(vectors) {
                            let _ = db.set_chunk_vector(*id, &model.id, &vector);
                        }
                    }
                }
                if let Ok(query) = self.runtime.embed(&model, prefs, &memory::query_text(message), cancel).await {
                    hits = self.store.lock().map_err(|_| "Dados ocupados")?.chunk_vectors(&model.id, &ids).unwrap_or_default().into_iter()
                        .filter_map(|(id, v)| memory::cosine(&query, &v).filter(|s| *s >= iris::SEMANTIC_MIN).map(|s| (id.to_string(), s)))
                        .collect();
                    hits.sort_by(|a, b| b.1.total_cmp(&a.1));
                    hits.truncate(20);
                }
            }
            let chosen: Vec<i64> = memory::fuse(&hits, &words, iris::CONTEXT_CHUNKS).iter().filter_map(|id| id.parse().ok()).collect();
            let chunks = self.store.lock().map_err(|_| "Dados ocupados")?.chunks_by_ids(&chosen).map_err(|_| "Falha ao consultar arquivos")?;
            Ok(iris::within_budget(chunks))
        }
        .await;
        result.unwrap_or_default()
    }
    /// Extrai memórias das mensagens novas de uma conversa e as aplica. Numa conversa
    /// de projeto, compara e grava só no projeto: o que se aprende ali fica ali.
    async fn remember(&self, prefs: &Preferences, conversation: &str, origin: &str, cancel: &CancellationToken) -> Result<Vec<Fact>, String> {
        let (range, project) = {
            let db = self.store.lock().map_err(|_| "Dados ocupados")?;
            (db.extraction_range(conversation).map_err(|_| "Falha ao consultar conversa")?, db.conversation_project(conversation).map_err(|_| "Conversa não encontrada")?)
        };
        let scope = Scope::exact(project.as_deref());
        let Some((after, until)) = range else { return Ok(Vec::new()) };
        let embedder = self.memory_model(prefs).ok_or("Ative a memória para lembrar automaticamente")?;
        let models = self.models()?;
        let chat = models::choose(&models, Capability::Text, self.budget(prefs), prefs.context_tokens, None)?;
        let turns = self.store.lock().map_err(|_| "Dados ocupados")?.turns_between(conversation, after, until, 20).map_err(|_| "Falha ao consultar conversa")?;
        if turns.iter().any(|t| t.role == "user") {
            self.backfill(prefs, &embedder, cancel).await?;
            let said: String = turns.iter().filter(|t| t.role == "user").map(|t| t.content.as_str()).collect::<Vec<_>>().join("\n").chars().take(2000).collect();
            let probe = self.runtime.embed(&embedder, prefs, &memory::document_text(&said), cancel).await?;
            let related: Vec<Fact> = self.search(&embedder.id, &probe, &said, memory::RELATED_MIN, 10, scope)?.into_iter().map(|(f, _)| f).collect();
            let raw = self.runtime.complete_json(&chat, prefs, memory::extraction_request(&turns, &related), memory::extraction_schema(), cancel).await?;
            let mut extraction = memory::parse_extraction(&raw, &related)?;
            // Automático: nada temporário (valores, "em aberto", "hoje"). Pedido explícito vale.
            if origin != "explicit" {
                extraction.remember.retain(|f| !memory::temporary(&f.content));
            }
            let texts: Vec<String> = extraction.remember.iter().map(|f| memory::document_text(&f.content)).collect();
            let vectors = self.runtime.embed_many(&embedder, prefs, &texts, cancel).await?;
            if cancel.is_cancelled() {
                return Err("Tarefa interrompida".into());
            }
            let db = self.store.lock().map_err(|_| "Dados ocupados")?;
            let existing = db.vectors(&embedder.id, scope).map_err(|_| "Falha ao consultar memórias")?;
            let mut changed = Vec::new();
            for id in &extraction.forget {
                let _ = db.delete_fact(id);
            }
            for (new, vector) in extraction.remember.iter().zip(vectors) {
                let saved = match &new.replaces {
                    Some(id) if !extraction.forget.contains(id) => {
                        // Texto novo: a privacidade volta a duvidosa, salvo se for sensível.
                        let privacy = if new.privacy == Privacy::Sensitive { Privacy::Sensitive } else { Privacy::Uncertain };
                        db.update_fact(id, Some(&new.content), Some(privacy)).ok().map(|_| id.clone())
                    }
                    _ => {
                        let duplicate = existing.iter().any(|(_, v)| memory::cosine(&vector, v).is_some_and(|s| s >= memory::DUPLICATE_MIN));
                        if duplicate { None } else {
                            db.insert_fact(&new.content, &new.kind, new.privacy, origin, Some(conversation), project.as_deref()).ok().map(|f| f.id)
                        }
                    }
                };
                if let Some(id) = saved {
                    let _ = db.set_vector(&id, &embedder.id, &vector);
                    changed.push(id);
                }
            }
            db.set_memory_cursor(conversation, until).map_err(|_| "Falha ao registrar extração")?;
            return db.facts_by_ids(&changed).map_err(|_| "Falha ao consultar memórias".into());
        }
        self.store.lock().map_err(|_| "Dados ocupados")?.set_memory_cursor(conversation, until).map_err(|_| "Falha ao registrar extração")?;
        Ok(Vec::new())
    }
}
impl Drop for Core {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[tauri::command]
pub fn desktop_snapshot(state: tauri::State<'_, Core>) -> Result<Value, String> {
    let prefs = state.prefs()?;
    let hw = hardware::detect();
    let (metrics, projects) = {
        let db = state.store.lock().map_err(|_| "Dados ocupados")?;
        (db.metrics().map_err(|_| "Não foi possível ler métricas")?, db.projects().map_err(|_| "Não foi possível ler projetos")?)
    };
    let artifacts:Vec<_>=runtime::artifacts().into_iter().map(|a|json!({"id":a.id,"label":a.label,"bytes":a.bytes,"installed":runtime::find_binary(&state.root.join("runtimes").join(a.id),a.binary).is_some()})).collect();
    let recommended = if hw.ram_total_bytes < 12 << 30 {
        "light"
    } else if hw.ram_total_bytes < 32 << 30 {
        "moderate"
    } else {
        "maximum"
    };
    let budget = models::memory_budget(&hw, &prefs);
    let installed = state.models()?;
    let runtime_root = state.root.join("runtimes");
    let runtime_bytes = if runtime::llama_installed(&runtime_root) {
        0
    } else {
        runtime::artifacts()
            .into_iter()
            .find(|a| a.id == runtime::preferred_llama())
            .map(|a| a.bytes)
            .unwrap_or(0)
    };
    let catalog: Vec<_> = models::catalog().iter().map(|entry| json!({
        "id": entry.id, "name": entry.name, "summary": entry.summary, "capabilities": entry.capabilities,
        "download_bytes": entry.download_bytes() + runtime_bytes, "memory_bytes": entry.memory_bytes(),
        "installed": installed.iter().any(|m| m.catalog.as_deref() == Some(entry.id)),
        "fits": entry.fits(budget, prefs.context_tokens),
    })).collect();
    let recommended_model = models::recommended(budget, prefs.context_tokens).map(|e| e.id);
    Ok(
        json!({"system_prompt":prefs.system_prompt(),"preferences":prefs,"memory_budget_bytes":budget,"hardware":hw,"models":installed,"runtimes":artifacts,"worker":state.runtime.status(),"metrics":metrics,"recommended_profile":recommended,"catalog":catalog,"recommended_model":recommended_model,"setup":state.setup.lock().map(|s|s.clone()).unwrap_or(Value::Null),"projects":projects}),
    )
}
/// Instrução padrão da Ayla (não faz parte da personalidade editável).
pub const OPTIONS_GUIDE: &str = "\n\nQuando o pedido aceitar mais de uma boa resposta (receitas, recomendações, ideias, soluções, caminhos, produtos), ofereça de 3 a 4 opções distintas, ordenadas da melhor para a pior segundo o seu próprio julgamento, e diga em uma frase por que cada uma funciona melhor ou pior. Se a pessoa pedir uma quantidade, respeite-a. Para perguntas com uma única resposta correta, responda direto.";
#[tauri::command]
pub fn save_preferences(
    state: tauri::State<'_, Core>,
    mut preferences: Preferences,
) -> Result<(), String> {
    let store = state.preferences.lock().map_err(|_| "Configuração ocupada")?;
    if let Ok(saved) = store.load() {
        preferences.keep_managed(&saved);
    }
    store
        .save(&preferences)
        .map_err(|_| {
            "Não foi possível salvar: verifique os limites e a URL HTTPS do provedor".into()
        })
}
/// Guarda só a aparência (salva enquanto a pessoa ajusta cores e controles), sem sobrescrever
/// outras preferências salvas no meio-tempo.
#[tauri::command]
pub fn save_appearance(state: tauri::State<'_, Core>, appearance: super::preferences::Appearance) -> Result<(), String> {
    let store = state.preferences.lock().map_err(|_| "Configuração ocupada")?;
    let mut prefs = store.load().map_err(|_| "Não foi possível ler as configurações")?;
    if prefs.appearance == appearance { return Ok(()); }
    prefs.appearance = appearance;
    store.save(&prefs).map_err(|_| "Aparência fora dos limites suportados".into())
}
/// Guarda só o modo de resposta, sem sobrescrever outras preferências salvas no meio-tempo.
#[tauri::command]
pub fn set_response_source(state: tauri::State<'_, Core>, source: String) -> Result<(), String> {
    let store = state.preferences.lock().map_err(|_| "Configuração ocupada")?;
    let mut prefs = store.load().map_err(|_| "Não foi possível ler as configurações")?;
    if prefs.response_source == source { return Ok(()); }
    prefs.response_source = source;
    store.save(&prefs).map_err(|_| "Modo de resposta inválido".into())
}
#[tauri::command]
pub fn cloud_key(
    state: tauri::State<'_, Core>,
    key: String,
    remember: bool,
    endpoint: String,
) -> Result<(), String> {
    if state.prefs()?.cloud_endpoint != endpoint {
        return Err("O provedor foi alterado; salve e revise antes de usar a chave".into());
    }
    if endpoint.is_empty() {
        return Err("Salve o endereço HTTPS do provedor antes da chave".into());
    }
    if key.is_empty() {
        state
            .keys
            .lock()
            .map_err(|_| "Credencial ocupada")?
            .remove(&endpoint);
        return match keyring::Entry::new("dev.feit.ayla", &endpoint).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err("Chave removida da sessão, mas não foi possível confirmar a remoção no cofre do sistema".into()),
        };
    }
    if remember {
        keyring::Entry::new("dev.feit.ayla",&endpoint).and_then(|e|e.set_password(&key)).map_err(|_|"O cofre do sistema não está disponível. Você pode usar a chave somente nesta sessão.")?;
    }
    state
        .keys
        .lock()
        .map_err(|_| "Credencial ocupada")?
        .insert(endpoint, key);
    Ok(())
}
#[tauri::command]
pub async fn import_model(
    app: tauri::AppHandle,
    state: tauri::State<'_, Core>,
    name: String,
    capability: Capability,
) -> Result<Option<Model>, String> {
    let projector_app = app.clone();
    if capability == Capability::Speech {
        return Err("A Ayla responde em texto; modelos de voz não são mais usados".into());
    }
    let selected = tauri::async_runtime::spawn_blocking(move || app.dialog().file().blocking_pick_file())
    .await
    .map_err(|_| "Não foi possível abrir seleção de arquivo")?;
    let Some(selected) = selected else {
        return Ok(None);
    };
    let source = selected.into_path().map_err(|_| "Arquivo local inválido")?;
    let companion = if capability == Capability::Vision {
        let a = projector_app;
        let chosen = tauri::async_runtime::spawn_blocking(move || {
            a.dialog()
                .file()
                .set_title("Escolha o projetor multimodal compatível")
                .add_filter("GGUF", &["gguf"])
                .blocking_pick_file()
        })
        .await
        .map_err(|_| "Falha ao escolher projetor")?;
        let Some(chosen) = chosen else {
            return Ok(None);
        };
        Some(chosen.into_path().map_err(|_| "Projetor inválido")?)
    } else {
        None
    };
    let root = state.root.join("models");
    let model = tauri::async_runtime::spawn_blocking(move || {
        models::import(&source, companion.as_deref(), &root, name, capability)
    })
    .await
    .map_err(|_| "Falha ao importar modelo")?
    .map_err(|e| e.to_string())?;
    let definition = serde_json::to_string(&model).map_err(|_| "Modelo inválido")?;
    let saved = state
        .store
        .lock()
        .map_err(|_| "Dados ocupados".to_string())
        .and_then(|db| {
            db.save_model(&model.id, &definition)
                .map_err(|_| "Não foi possível registrar modelo".to_string())
        });
    if let Err(error) = saved {
        // A importação criou apenas esta cópia; a origem continua preservada.
        if let Some(directory) = model.path.parent() {
            let _ = fs::remove_dir_all(directory);
        }
        return Err(error);
    }
    Ok(Some(model))
}
#[tauri::command]
pub fn remove_model(state: tauri::State<'_, Core>, id: String) -> Result<(), String> {
    let _gate = state
        .runtime
        .gate
        .try_lock()
        .map_err(|_| "Interrompa a tarefa atual antes de remover modelos")?;
    state.runtime.stop_all();
    let model = state
        .models()?
        .into_iter()
        .find(|m| m.id == id)
        .ok_or("Modelo não encontrado")?;
    let directory = model.path.parent().ok_or("Pasta inválida")?;
    if directory.parent() != Some(state.root.join("models").as_path()) {
        return Err("Pasta inválida".into());
    }
    fs::remove_dir_all(directory).map_err(|_| "Não foi possível remover cópia do modelo")?;
    state
        .store
        .lock()
        .map_err(|_| "Dados ocupados")?
        .remove_model(&id)
        .map_err(|_| "Não foi possível remover registro")?;
    Ok(())
}
#[tauri::command]
pub async fn install_runtime(
    state: tauri::State<'_, Core>,
    id: String,
    request_id: String,
    events: Channel<Value>,
) -> Result<(), String> {
    let _install = state
        .install_gate
        .try_lock()
        .map_err(|_| "Outra instalação está em andamento")?;
    let token = state.job(&request_id)?;
    let result = runtime::install(&state.root.join("runtimes"), &id, &token, |e| {
        let _ = events.send(e);
    })
    .await;
    state.finish(&request_id);
    result
}
/// Prepara um modelo do catálogo: instala o llama.cpp se faltar e baixa os
/// arquivos verificados. Só é chamado após o usuário confirmar o tamanho.
/// Identificador da tarefa de preparo automático (para interromper pela interface).
pub const SETUP_JOB: &str = "auto-setup";

impl Core {
    /// Componentes que faltam para usar o perfil principal: llama.cpp, o melhor
    /// modelo de conversa que cabe no orçamento e, com memória ligada, o de memória.
    fn default_plan(&self, prefs: &Preferences) -> Result<Vec<&'static models::CatalogEntry>, String> {
        let installed = self.models()?;
        let mut plan = Vec::new();
        if !installed.iter().any(|m| m.supports(Capability::Text)) {
            if let Some(entry) = models::recommended(self.budget(prefs), prefs.context_tokens) {
                plan.push(entry);
            }
        }
        let missing = |kind: Capability| !installed.iter().any(|m| m.supports(kind));
        let first = |kind: Capability| models::catalog().iter().find(|e| e.capabilities.contains(&kind));
        if prefs.memory_auto && missing(Capability::Embedding) {
            plan.extend(first(Capability::Embedding));
        }
        if prefs.voice_enabled() {
            let catalog_id = |id: &str| installed.iter().any(|m| m.catalog.as_deref() == Some(id));
            let main_transcriber = installed.iter().any(|m| m.supports(Capability::Transcription) && m.catalog.as_deref() != Some(PREVIEW_TRANSCRIBER));
            if !main_transcriber {
                plan.extend(models::catalog().iter().find(|e| e.capabilities.contains(&Capability::Transcription) && e.id != PREVIEW_TRANSCRIBER));
            }
            if !catalog_id(PREVIEW_TRANSCRIBER) {
                plan.extend(models::catalog().iter().find(|e| e.id == PREVIEW_TRANSCRIBER));
            }
        }
        Ok(plan)
    }
    /// Instala as entradas na ordem, com o llama.cpp antes se faltar; progresso acumulado.
    async fn install_entries(&self, entries: &[&'static models::CatalogEntry], token: &CancellationToken, mut progress: impl FnMut(&str, u64, u64)) -> Result<(), String> {
        let runtimes = self.root.join("runtimes");
        // Motores que faltam para as entradas pedidas (llama.cpp, whisper.cpp).
        let mut needed: Vec<runtime::Artifact> = Vec::new();
        for entry in entries {
            let id = if entry.runtime == "llama" { runtime::preferred_llama() } else { entry.runtime };
            let installed = if entry.runtime == "llama" { runtime::llama_installed(&runtimes) } else {
                runtime::artifacts().iter().any(|a| a.id == id && runtime::find_binary(&runtimes.join(id), a.binary).is_some())
            };
            if !installed && !needed.iter().any(|a| a.id == id) {
                needed.extend(runtime::artifacts().into_iter().find(|a| a.id == id));
            }
        }
        let runtime_bytes: u64 = needed.iter().map(|a| a.bytes).sum();
        let total = runtime_bytes + entries.iter().map(|e| e.download_bytes()).sum::<u64>();
        let mut done = 0u64;
        for artifact in &needed {
            runtime::install(&runtimes, artifact.id, token, |event| {
                progress("Motores de IA", done + event["received"].as_u64().unwrap_or(0), total)
            })
            .await?;
            done += artifact.bytes;
        }
        let models_root = self.root.join("models");
        fs::create_dir_all(&models_root).map_err(|_| "Não foi possível criar a pasta de modelos")?;
        // Restos de downloads interrompidos por encerramento abrupto; nunca registrados.
        if let Ok(items) = fs::read_dir(&models_root) {
            for item in items.flatten() {
                if item.file_name().to_string_lossy().ends_with(".installing") {
                    let _ = fs::remove_dir_all(item.path());
                }
            }
        }
        for entry in entries {
            if self.models()?.iter().any(|m| m.catalog.as_deref() == Some(entry.id)) {
                done += entry.download_bytes();
                continue;
            }
            let model = runtime::install_model(&models_root, entry, token, |received| progress(entry.name, done + received, total)).await?;
            done += entry.download_bytes();
            let definition = serde_json::to_string(&model).map_err(|_| "Modelo inválido")?;
            let saved = self.store.lock().map_err(|_| "Dados ocupados".to_string())
                .and_then(|db| db.save_model(&model.id, &definition).map_err(|_| "Não foi possível registrar modelo".to_string()));
            if let Err(error) = saved {
                if let Some(directory) = model.path.parent() {
                    let _ = fs::remove_dir_all(directory);
                }
                return Err(error);
            }
        }
        Ok(())
    }
    fn set_setup(&self, value: Value) {
        if let Ok(mut setup) = self.setup.lock() {
            *setup = value;
        }
    }
    /// Preparo do perfil principal sem cliques. Chamado ao abrir o app (uma vez por
    /// sessão) e pelo botão "Tentar novamente". Nunca roda no modo somente nuvem.
    /// Ao terminar ou falhar, avisa pelo sistema se a janela estiver fora de foco.
    pub async fn prepare_defaults(&self, app: &tauri::AppHandle) -> Result<(), String> {
        let prefs = self.prefs()?;
        if prefs.usage_mode == UsageMode::CloudOnly {
            return Ok(());
        }
        if self.default_plan(&prefs)?.is_empty() {
            return Ok(());
        }
        let Ok(_install) = self.install_gate.try_lock() else { return Ok(()) };
        let token = self.job(SETUP_JOB)?;
        self.set_setup(json!({"active":true,"label":"Preparando","received":0,"total":0}));
        // Repete enquanto houver o que preparar: a pessoa pode ativar a voz no meio do preparo.
        let mut result = Ok(());
        for _ in 0..3 {
            let plan = match self.prefs().and_then(|p| self.default_plan(&p)) {
                Ok(plan) if plan.is_empty() => break,
                Ok(plan) => plan,
                Err(error) => { result = Err(error); break; }
            };
            result = self.install_entries(&plan, &token, |label, received, total| {
                self.set_setup(json!({"active":true,"label":label,"received":received,"total":total}));
            }).await;
            if result.is_err() {
                break;
            }
        }
        self.finish(SETUP_JOB);
        self.set_setup(match &result {
            Ok(()) => json!({"active":false}),
            Err(_) if token.is_cancelled() => json!({"active":false,"cancelled":true}),
            Err(error) => json!({"active":false,"error":error}),
        });
        self.notify_setup(app, &result, &token);
        result
    }
    fn notify_setup(&self, app: &tauri::AppHandle, result: &Result<(), String>, token: &CancellationToken) {
        let event = match result {
            Ok(()) => notify::Event::SetupDone,
            Err(_) if token.is_cancelled() => return,
            Err(_) => notify::Event::SetupFailed,
        };
        if let Ok(prefs) = self.prefs() {
            notify::send(app, &prefs, event);
        }
    }
}

#[tauri::command]
pub async fn setup_start(app: tauri::AppHandle, state: tauri::State<'_, Core>) -> Result<(), String> {
    state.prepare_defaults(&app).await
}

#[tauri::command]
pub async fn install_catalog_model(
    app: tauri::AppHandle,
    state: tauri::State<'_, Core>,
    id: String,
    request_id: String,
    events: Channel<Value>,
) -> Result<(), String> {
    let entry = models::catalog()
        .iter()
        .find(|e| e.id == id)
        .ok_or("Modelo fora do catálogo")?;
    let _install = state
        .install_gate
        .try_lock()
        .map_err(|_| "Outra instalação está em andamento")?;
    if state.models()?.iter().any(|m| m.catalog.as_deref() == Some(entry.id)) {
        return Ok(());
    }
    let token = state.job(&request_id)?;
    let result = state.install_entries(&[entry], &token, |_, received, total| {
        let _ = events.send(json!({"type":"download","received":received,"total":total}));
    }).await;
    state.finish(&request_id);
    state.notify_setup(&app, &result, &token);
    result
}
#[tauri::command]
pub fn cancel_job(state: tauri::State<'_, Core>, request_id: String) -> Result<(), String> {
    if let Some(token) = state
        .jobs
        .lock()
        .map_err(|_| "Tarefas ocupadas")?
        .get(&request_id)
    {
        token.cancel();
    }
    Ok(())
}
#[tauri::command]
pub fn unload_models(state: tauri::State<'_, Core>) -> Result<(), String> {
    let _gate = state
        .runtime
        .gate
        .try_lock()
        .map_err(|_| "Interrompa a tarefa atual antes de liberar o modelo")?;
    state.runtime.stop_all();
    Ok(())
}
#[tauri::command]
pub fn conversations(state: tauri::State<'_, Core>, project: Option<String>) -> Result<Value, String> {
    Ok(json!(state
        .store
        .lock()
        .map_err(|_| "Dados ocupados")?
        .conversations(project.as_deref())
        .map_err(|_| "Falha ao ler conversas")?))
}
#[tauri::command]
pub fn create_conversation(state: tauri::State<'_, Core>, project: Option<String>) -> Result<Value, String> {
    let db = state.store.lock().map_err(|_| "Dados ocupados")?;
    if let Some(id) = &project {
        db.project(id).ok().flatten().ok_or("Projeto não encontrado")?;
    }
    Ok(json!(db.create_conversation(DEFAULT_TITLE, project.as_deref()).map_err(|_| "Falha ao criar conversa")?))
}

/// Novo título escolhido pela pessoa.
#[tauri::command]
pub fn rename_conversation(state: tauri::State<'_, Core>, id: String, title: String) -> Result<(), String> {
    let count = title.trim().chars().count();
    if count == 0 || count > 120 {
        return Err("Dê à conversa um título de até 120 caracteres".into());
    }
    match state.store.lock().map_err(|_| "Dados ocupados")?.rename_conversation(&id, &title) {
        Ok(true) => Ok(()),
        _ => Err("Conversa não encontrada".into()),
    }
}
/// Apaga a conversa, as mensagens e os arquivos dela (a interface pede confirmação).
#[tauri::command]
pub fn delete_conversation(state: tauri::State<'_, Core>, id: String) -> Result<(), String> {
    let copies = state.store.lock().map_err(|_| "Dados ocupados")?.delete_conversation(&id).map_err(|_| "Não foi possível apagar a conversa")?.ok_or("Conversa não encontrada")?;
    for path in copies {
        remove_copy(&state.root, &path);
    }
    Ok(())
}

#[tauri::command]
pub fn projects(state: tauri::State<'_, Core>) -> Result<Value, String> {
    Ok(json!(state.store.lock().map_err(|_| "Dados ocupados")?.projects().map_err(|_| "Falha ao ler projetos")?))
}
fn project_fields(name: &str, description: &str) -> Result<(), String> {
    let name = name.trim().chars().count();
    if name == 0 || name > 80 {
        return Err("Dê ao projeto um nome de até 80 caracteres".into());
    }
    if description.trim().chars().count() > 2000 {
        return Err("A descrição do projeto deve ter até 2000 caracteres".into());
    }
    Ok(())
}
#[tauri::command]
pub fn create_project(state: tauri::State<'_, Core>, name: String, description: String) -> Result<Value, String> {
    project_fields(&name, &description)?;
    Ok(json!(state.store.lock().map_err(|_| "Dados ocupados")?.create_project(&name, &description).map_err(|_| "Não foi possível criar o projeto")?))
}
#[tauri::command]
pub fn update_project(state: tauri::State<'_, Core>, id: String, name: String, description: String) -> Result<(), String> {
    project_fields(&name, &description)?;
    state.store.lock().map_err(|_| "Dados ocupados")?.update_project(&id, &name, &description).map_err(|_| "Projeto não encontrado".into())
}
/// Apaga o projeto com conversas e memórias dele (a interface pede confirmação).
#[tauri::command]
pub fn delete_project(state: tauri::State<'_, Core>, id: String) -> Result<(), String> {
    // Uma pasta protegida bloqueada não sai junto com o projeto: precisa ser aberta antes.
    if let Some(folder) = locked_inside(&state.root.join("projetos-notas").join(&id)) {
        return Err(format!("As notas do projeto têm a pasta protegida \"{folder}\", que está bloqueada. Abra-a na Athena com a senha antes de apagar o projeto."));
    }
    let db = state.store.lock().map_err(|_| "Dados ocupados")?;
    let copies = db.project_file_paths(&id).unwrap_or_default();
    let name = db.projects().unwrap_or_default().into_iter().find(|p| p.id == id).map(|p| p.name).unwrap_or_else(|| "Projeto".into());
    match db.delete_project(&id) {
        Ok(true) => {
            for path in copies {
                remove_copy(&state.root, &path);
            }
            // Notas do projeto (Athena): vão para a lixeira das notas da Athena, não somem.
            let notes = state.root.join("projetos-notas").join(&id);
            if notes.is_dir() {
                let bin = state.root.join("athena").join(".trash");
                let _ = fs::create_dir_all(&bin);
                let mut target = bin.join(format!("Projeto {}", athena::file_name(&name)));
                let mut n = 2;
                while target.exists() {
                    target = bin.join(format!("Projeto {} {n}", athena::file_name(&name)));
                    n += 1;
                }
                let _ = fs::rename(&notes, &target);
            }
            Ok(())
        }
        _ => Err("Projeto não encontrado".into()),
    }
}
#[tauri::command]
pub fn conversation_messages(state: tauri::State<'_, Core>, id: String) -> Result<Value, String> {
    Ok(json!(state
        .store
        .lock()
        .map_err(|_| "Dados ocupados")?
        .messages(&id)
        .map_err(|_| "Falha ao ler conversa")?))
}
#[tauri::command]
pub fn conversation_consent(
    state: tauri::State<'_, Core>,
    id: String,
    allowed: bool,
) -> Result<(), String> {
    state
        .store
        .lock()
        .map_err(|_| "Dados ocupados")?
        .set_consent(&id, allowed)
        .map_err(|_| "Falha ao salvar permissão".into())
}

#[derive(Deserialize)]
pub struct ChatRequest {
    pub id: String,
    pub conversation: String,
    pub message: String,
    pub personality_preview: Option<String>,
    pub capability: Option<Capability>,
    pub model: Option<String>,
    pub sensitivity: Sensitivity,
    pub reviewed: bool,
    pub request_cloud: bool,
    #[serde(default)]
    pub web_search: bool,
    #[serde(default)]
    pub automatic: bool,
    /// Automático: resposta da pessoa quando a Ayla perguntou ("cloud" ou "local"), ou a rota já
    /// verificada antes do envio. Os bloqueios locais valem de novo aqui.
    #[serde(default)]
    pub auto_route: Option<String>,
    /// Arquivos anexados (ids devolvidos por `add_file`), no máximo 5.
    #[serde(default)]
    pub attachments: Vec<String>,
    /// Modelo da nuvem escolhido para esta resposta (vazio: o de Configurações › Nuvem).
    #[serde(default)]
    pub cloud_model: Option<String>,
    /// Esforço de raciocínio na nuvem: "low", "medium" ou "high" (vazio: padrão da plataforma).
    #[serde(default)]
    pub effort: Option<String>,
    /// Memórias públicas que a pessoa manteve marcadas na revisão de envio à nuvem.
    #[serde(default)]
    pub references: Vec<memory::Reference>,
}
pub const DEFAULT_TITLE: &str = "Nova conversa";
/// Título provisório: primeiras palavras do pedido, sem quebras nem pontuação final.
pub fn quick_title(message: &str) -> String {
    let words: Vec<&str> = message.split_whitespace().collect();
    let mut title = words.iter().take(6).copied().collect::<Vec<_>>().join(" ");
    title = title.trim_end_matches(|c: char| !c.is_alphanumeric()).to_string();
    if title.chars().count() > 48 {
        title = title.chars().take(47).collect::<String>().trim_end().to_string() + "…";
    } else if words.len() > 6 {
        title.push('…');
    }
    let mut chars = title.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => DEFAULT_TITLE.into(),
    }
}
/// Pedido ao modelo local para um título curto (JSON com o campo `titulo`).
fn title_request(message: &str) -> Vec<Value> {
    let excerpt: String = message.chars().take(1200).collect();
    vec![
        json!({"role":"system","content":"Você dá nomes curtos a conversas. Responda em português do Brasil com um título de 2 a 6 palavras que resuma o pedido, sem aspas, sem ponto final e sem emojis."}),
        json!({"role":"user","content":format!("Primeiro pedido da conversa:\n{excerpt}")}),
    ]
}
fn title_schema() -> Value {
    json!({"type":"object","properties":{"titulo":{"type":"string","maxLength":60}},"required":["titulo"],"additionalProperties":false})
}
/// Título válido a partir da resposta do modelo; `None` se vier vazio ou estranho.
pub fn clean_title(raw: &str) -> Option<String> {
    let value: Value = serde_json::from_str(raw).ok()?;
    let title = value["titulo"].as_str()?.lines().next()?.trim().trim_matches(|c: char| matches!(c, '"' | '\'' | '“' | '”' | '*' | '#') || c.is_whitespace());
    let title = title.trim_end_matches(['.', '!', ';', ':']).trim();
    let count = title.chars().count();
    if count < 2 || count > 60 || title.split_whitespace().count() > 10 {
        return None;
    }
    let mut chars = title.chars();
    chars.next().map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
}
/// Resume o primeiro pedido com o modelo local e troca o título provisório.
/// Sem modelo local (ou se a pessoa voltar a conversar), fica o provisório.
async fn smart_title(app: tauri::AppHandle, conversation: String, message: String, quick: String) {
    use tauri::{Emitter, Manager};
    let Some(state) = app.try_state::<Core>() else { return };
    let Ok(prefs) = state.prefs() else { return };
    let Ok(models_list) = state.models() else { return };
    let Ok(model) = models::choose(&models_list, Capability::Text, state.budget(&prefs), prefs.context_tokens, None) else { return };
    // Espera a resposta liberar o modelo; uma nova mensagem cancela este título.
    let Ok(_gate) = tokio::time::timeout(Duration::from_secs(30), state.runtime.gate.lock()).await else { return };
    let token = CancellationToken::new();
    if let Ok(mut slot) = state.background.lock() {
        *slot = Some(token.clone());
    }
    let raw = state.runtime.complete_json(&model, &prefs, title_request(&message), title_schema(), &token).await;
    if let Ok(mut slot) = state.background.lock() {
        *slot = None;
    }
    let Some(title) = raw.ok().and_then(|raw| clean_title(&raw)) else { return };
    if state.store.lock().ok().and_then(|db| db.retitle(&conversation, &quick, &title).ok()) == Some(true) {
        let _ = app.emit("conversation-updated", json!({"id":conversation,"title":title}));
    }
}

/// Contexto do projeto no prompt local: nome e "sobre o projeto".
fn project_block(project: &store::Project) -> String {
    let mut block = format!("\n\nEsta conversa faz parte do projeto \"{}\".", project.name);
    if !project.description.trim().is_empty() {
        block.push_str(&format!(" Sobre o projeto: {}", project.description.trim()));
    }
    block
}

// Athena: notas pessoais (comandos da interface). Toda nota é (raiz, caminho relativo).

#[derive(Clone, Debug, Serialize)]
pub struct AthenaRoot {
    /// "ayla", "vault:<pasta>" ou "project:<id>".
    pub id: String,
    pub kind: &'static str,
    pub name: String,
    pub path: PathBuf,
    pub available: bool,
}

fn project_root_id(id: &str) -> String {
    format!("project:{id}")
}

/// Árvore da Athena: cada raiz com as pastas e as notas, e os cofres do Obsidian ainda não abertos.
#[tauri::command]
pub async fn athena_tree(state: tauri::State<'_, Core>) -> Result<Value, String> {
    let prefs = state.prefs()?;
    let roots = state.athena_roots(&prefs);
    let open: Vec<String> = prefs.vaults();
    let listed = tauri::async_runtime::spawn_blocking(move || {
        roots.into_iter().map(|root| {
            let (mut folders, mut notes, mut files) = if root.path.is_dir() { (athena::folders(&root.path), athena::list(&root.path), athena::attachments(&root.path)) } else { (Vec::new(), Vec::new(), Vec::new()) };
            let mut locks = Vec::new();
            for folder in athena_lock::protected_folders(&root.path) {
                let dir = root.path.join(&folder);
                let session = athena_lock::session(&dir);
                locks.push(json!({"folder": folder, "unlocked": session.is_some(), "since": session.as_ref().map(|s| s.1), "recovery": athena_lock::has_recovery(&dir)}));
                // Desbloqueada: o conteúdo aparece na árvore da Athena (e só nela).
                let Some((key, _)) = session else { continue };
                for entry in athena_lock::entries(&dir, &key) {
                    let path = format!("{folder}/{}", entry.path);
                    let title = std::path::Path::new(&entry.path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                    let note = athena::Note { path: path.clone(), title, modified: entry.modified, bytes: entry.bytes };
                    if entry.kind == "folder" {
                        folders.push(path);
                    } else if path.to_lowercase().ends_with(".md") {
                        notes.push(note);
                    } else if athena::attachment_mime(std::path::Path::new(&path)).is_some() {
                        files.push(note);
                    }
                }
            }
            json!({
                "id": root.id, "kind": root.kind, "name": root.name, "path": root.path.to_string_lossy(), "available": root.available,
                "obsidian": root.path.join(".obsidian").is_dir(), "git": athena_git::is_repo(&root.path),
                "folders": folders, "notes": notes, "files": files, "protected": locks, "bannerHeight": athena::make_md_banner_height(&root.path),
            })
        }).collect::<Vec<_>>()
    }).await.map_err(|_| "Falha ao ler as notas")?;
    let suggested: Vec<String> = athena::obsidian_vaults().into_iter().filter(|v| !open.contains(v)).collect();
    Ok(json!({"roots": listed, "suggested": suggested, "git_enabled": prefs.athena_git}))
}

/// Abre um cofre na Athena (pasta existente, ex.: do Obsidian).
#[tauri::command]
pub fn athena_add_vault(state: tauri::State<'_, Core>, path: String) -> Result<(), String> {
    let folder = PathBuf::from(&path);
    if !folder.is_absolute() || !folder.is_dir() {
        return Err("Escolha uma pasta que exista neste computador".into());
    }
    if folder.starts_with(&state.root) {
        return Err("Essa pasta já é da Ayla".into());
    }
    let mut prefs = state.prefs()?;
    let mut vaults = prefs.vaults();
    if !vaults.contains(&path) {
        vaults.push(path);
    }
    prefs.athena_vaults = vaults;
    prefs.athena_folder.clear();
    state.preferences.lock().map_err(|_| "Configuração ocupada")?.save(&prefs).map_err(|_| "Não foi possível salvar o cofre".into())
}

/// Tira o cofre da Athena (os arquivos continuam onde estão).
#[tauri::command]
pub fn athena_remove_vault(state: tauri::State<'_, Core>, path: String) -> Result<(), String> {
    let mut prefs = state.prefs()?;
    prefs.athena_vaults = prefs.vaults().into_iter().filter(|v| *v != path).collect();
    prefs.athena_folder.clear();
    state.preferences.lock().map_err(|_| "Configuração ocupada")?.save(&prefs).map_err(|_| "Não foi possível salvar".into())
}

/// Escolher uma pasta (para abrir como cofre). Só devolve o caminho.
#[tauri::command]
pub async fn athena_pick_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let handle = app.clone();
    crate::desktop::panel_dialog(&handle, move || {
        app.dialog().file().blocking_pick_folder().and_then(|p| p.into_path().ok()).map(|p| p.to_string_lossy().into_owned())
    })
    .await
}

#[tauri::command]
pub async fn athena_read(state: tauri::State<'_, Core>, root: String, path: String) -> Result<Value, String> {
    let dir = state.athena_path(&state.prefs()?, &root)?;
    if let Some(place) = protected(&dir, &path) {
        let key = unlocked(&place)?;
        let (bytes, entry) = tauri::async_runtime::spawn_blocking(move || athena_lock::read(&place.dir, &key, &place.inner))
            .await.map_err(|_| "Falha ao ler a nota")??;
        return Ok(json!({"content": String::from_utf8_lossy(&bytes), "modified": entry.modified}));
    }
    let (content, modified) = athena::read(&dir, &path)?;
    Ok(json!({"content": content, "modified": modified}))
}

/// Grava a nota; recusa se ela mudou fora da Ayla desde `expected`. Devolve a nova data.
#[tauri::command]
pub async fn athena_write(state: tauri::State<'_, Core>, root: String, path: String, content: String, expected: Option<i64>) -> Result<i64, String> {
    let dir = state.athena_path(&state.prefs()?, &root)?;
    if let Some(place) = protected(&dir, &path) {
        let key = unlocked(&place)?;
        protected_note(&place.inner)?;
        return tauri::async_runtime::spawn_blocking(move || athena_lock::write(&place.dir, &key, &place.inner, content.as_bytes(), expected).map(|e| e.modified))
            .await.map_err(|_| "Falha ao salvar a nota")?;
    }
    athena::write(&dir, &path, &content, expected)
}

#[tauri::command]
pub async fn athena_paste_image(app: tauri::AppHandle, state: tauri::State<'_, Core>, root: String, path: String, before: String, after: String, expected: i64, data: String) -> Result<Value, String> {
    use tauri::Emitter;
    let dir = state.athena_path(&state.prefs()?, &root)?;
    let result = tauri::async_runtime::spawn_blocking(move || {
        if data.len() > (athena::MAX_ATTACHMENT as usize).div_ceil(3) * 4 {
            return Err("A imagem passa de 15 MB".into());
        }
        let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|_| "Imagem inválida")?;
        if let Some(place) = protected(&dir, &path) {
            return protected_paste(&place, &before, &after, expected, &bytes);
        }
        athena::paste_image(&dir, &path, &before, &after, expected, &bytes)
    }).await.map_err(|_| "Falha ao colar a imagem")??;
    let _ = app.emit("notes-updated", json!({"root": root}));
    Ok(json!({"embed": result.0, "modified": result.1}))
}

#[tauri::command]
pub async fn athena_set_banner(app: tauri::AppHandle, state: tauri::State<'_, Core>, root: String, path: String, content: String, expected: i64, image: String) -> Result<Value, String> {
    use tauri::Emitter;
    let dir = state.athena_path(&state.prefs()?, &root)?;
    let (content, modified) = tauri::async_runtime::spawn_blocking(move || match protected(&dir, &path) {
        Some(place) => protected_banner(&dir, &place, &content, expected, &image),
        None => athena::set_banner(&dir, &path, &content, expected, &image),
    })
        .await.map_err(|_| "Falha ao definir o banner")??;
    let _ = app.emit("notes-updated", json!({"root": root}));
    Ok(json!({"content": content, "modified": modified}))
}

/// Nota nova numa pasta da raiz (vazia = raiz).
#[tauri::command]
pub async fn athena_create(state: tauri::State<'_, Core>, root: String, folder: Option<String>, title: String, empty: Option<bool>) -> Result<String, String> {
    let dir = state.athena_path(&state.prefs()?, &root)?;
    let title = athena::file_name(&title);
    // Nota nova pelo botão: vazia, como no Obsidian (o título é o nome do arquivo).
    let content = if empty.unwrap_or(false) { String::new() } else { format!("# {title}\n\n") };
    let folder = folder.unwrap_or_default();
    if let Some(place) = protected(&dir, &folder) {
        let key = unlocked(&place)?;
        let taken: Vec<String> = athena_lock::entries(&place.dir, &key).into_iter().map(|e| e.path.to_lowercase()).collect();
        let inside = |name: &str| if place.inner.is_empty() { name.to_string() } else { format!("{}/{name}", place.inner) };
        let mut inner = inside(&format!("{title}.md"));
        let mut n = 2;
        while taken.contains(&inner.to_lowercase()) {
            inner = inside(&format!("{title} {n}.md"));
            n += 1;
        }
        athena_lock::write(&place.dir, &key, &inner, content.as_bytes(), None)?;
        return Ok(format!("{}/{inner}", place.folder));
    }
    athena::create_in(&dir, &folder, &title, &content)
}

#[tauri::command]
pub async fn athena_mkdir(state: tauri::State<'_, Core>, root: String, parent: Option<String>, name: String) -> Result<String, String> {
    let dir = state.athena_path(&state.prefs()?, &root)?;
    let parent = parent.unwrap_or_default();
    if let Some(place) = protected(&dir, &parent) {
        let key = unlocked(&place)?;
        let name = athena::file_name(&name);
        let inner = if place.inner.is_empty() { name } else { format!("{}/{name}", place.inner) };
        athena_lock::add_folder(&place.dir, &key, &inner)?;
        return Ok(format!("{}/{inner}", place.folder));
    }
    athena::mkdir(&dir, &parent, &name)
}

/// Nota de hoje (AAAA-MM-DD.md) na raiz indicada, criada se ainda não existir.
#[tauri::command]
pub async fn athena_daily(state: tauri::State<'_, Core>, root: String) -> Result<String, String> {
    let dir = state.athena_path(&state.prefs()?, &root)?;
    athena::daily(&dir, reminders::now_local().date())
}

/// Move a nota para a lixeira da raiz (`.trash`).
#[tauri::command]
pub async fn athena_trash(state: tauri::State<'_, Core>, root: String, path: String) -> Result<(), String> {
    let dir = state.athena_path(&state.prefs()?, &root)?;
    if let Some(place) = protected(&dir, &path) {
        let key = unlocked(&place)?;
        return athena_lock::trash(&place.dir, &key, &place.inner, &dir.join(".trash"));
    }
    athena::trash(&dir, &path)
}

/// Renomeia ou move a nota (caminho novo, com pastas) e atualiza os links das outras notas.
#[tauri::command]
pub async fn athena_rename(state: tauri::State<'_, Core>, root: String, path: String, to: String) -> Result<Value, String> {
    let dir = state.athena_path(&state.prefs()?, &root)?;
    let to = if to.to_lowercase().ends_with(".md") { to } else { format!("{to}.md") };
    if protected(&dir, &path).is_some() || protected(&dir, &to).is_some() {
        let name = std::path::Path::new(&to).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        if athena::file_name(&name) != name {
            return Err("O nome não pode ter / \\ : * ? \" < > | # ^ [ ]".into());
        }
        let new = tauri::async_runtime::spawn_blocking(move || move_across(&dir, &path, &to)).await.map_err(|_| "Falha ao renomear")??;
        return Ok(json!({"path": new, "relinked": 0}));
    }
    let (new, changed) = tauri::async_runtime::spawn_blocking(move || athena::rename_note(&dir, &path, &to)).await.map_err(|_| "Falha ao renomear")??;
    Ok(json!({"path": new, "relinked": changed}))
}

/// Renomeia ou move uma pasta e atualiza os links das notas dela.
#[tauri::command]
pub async fn athena_rename_folder(state: tauri::State<'_, Core>, root: String, path: String, to: String) -> Result<Value, String> {
    let dir = state.athena_path(&state.prefs()?, &root)?;
    let (from_place, to_place) = (protected(&dir, &path), protected(&dir, &to));
    match (&from_place, &to_place) {
        // A própria pasta protegida muda de nome ou de lugar: os arquivos cifrados vão junto.
        (Some(place), None) if place.inner.is_empty() => {
            let session = athena_lock::session(&place.dir).ok_or("Pasta protegida: digite a senha na Athena para abri-la")?;
            let old = place.dir.clone();
            let (new, changed) = tauri::async_runtime::spawn_blocking(move || athena::rename_folder(&dir, &path, &to)).await.map_err(|_| "Falha ao renomear")??;
            athena_lock::end_session(&old);
            athena_lock::start_session(&state.athena_path(&state.prefs()?, &root)?.join(&new), session.0);
            return Ok(json!({"path": new, "relinked": changed}));
        }
        (Some(from), Some(target)) if from.dir == target.dir && !from.inner.is_empty() && !target.inner.is_empty() => {
            let key = unlocked(from)?;
            athena_lock::rename(&from.dir, &key, &from.inner, &target.inner)?;
            return Ok(json!({"path": to, "relinked": 0}));
        }
        (None, None) => {}
        _ => return Err("Pastas inteiras ainda não entram nem saem de uma pasta protegida; mova as notas".into()),
    }
    let (new, changed) = tauri::async_runtime::spawn_blocking(move || athena::rename_folder(&dir, &path, &to)).await.map_err(|_| "Falha ao renomear")??;
    Ok(json!({"path": new, "relinked": changed}))
}

/// Tags e conexões das notas, extraídas na mesma passagem pelos arquivos.
#[tauri::command]
pub async fn athena_overview(state: tauri::State<'_, Core>) -> Result<Value, String> {
    let roots: Vec<AthenaRoot> = state.athena_roots(&state.prefs()?).into_iter().filter(|r| r.path.is_dir()).collect();
    tauri::async_runtime::spawn_blocking(move || athena_overview_for(roots))
        .await.map_err(|_| "Falha ao ler as conexões das notas".into())
}

fn athena_overview_for(roots: Vec<AthenaRoot>) -> Value {
    use std::collections::{BTreeMap, BTreeSet, HashMap};
    let mut tags = BTreeMap::<String, Vec<Value>>::new();
    let mut nodes = Vec::new();
    let mut edges = BTreeSet::new();
    for root in roots {
        let mut notes = athena::list(&root.path);
        notes.sort_unstable_by(|a, b| a.path.cmp(&b.path));
        let base = nodes.len();
        let mut paths = HashMap::<String, usize>::new();
        for (index, note) in notes.iter().enumerate() {
            nodes.push(json!({"root": root.id, "rootName": root.name, "path": note.path, "title": note.title}));
            let path = note.path.to_lowercase();
            let path = path.trim_end_matches(".md");
            for suffix in std::iter::once(path).chain(path.match_indices('/').map(|(i, _)| &path[i + 1..])) {
                paths.entry(suffix.to_owned()).and_modify(|other| {
                    let previous = &notes[*other];
                    if note.path.len() < previous.path.len()
                        || (note.path.len() == previous.path.len() && note.modified > previous.modified) {
                        *other = index;
                    }
                }).or_insert(index);
            }
        }
        for (index, note) in notes.iter().enumerate() {
            let Ok((text, _)) = athena::read(&root.path, &note.path) else { continue };
            for tag in athena::tags(&text) {
                tags.entry(tag).or_default().push(nodes[base + index].clone());
            }
            for target in athena::graph_targets(&text) {
                let target = target.to_lowercase();
                if let Some(&other) = paths.get(target.trim_end_matches(".md")).filter(|&&other| other != index) {
                    edges.insert([base + index.min(other), base + index.max(other)]);
                }
            }
        }
    }
    let tags: Vec<_> = tags.into_iter().map(|(tag, notes)| json!({"tag": tag, "notes": notes})).collect();
    json!({"tags": tags, "graph": {"nodes": nodes, "edges": edges}})
}

/// Imagem anexada a uma nota (![[foto.png]]), procurada pelo nome na raiz, como data URL.
#[tauri::command]
pub async fn athena_attachment(state: tauri::State<'_, Core>, root: String, target: String) -> Result<Option<String>, String> {
    let dir = state.athena_path(&state.prefs()?, &root)?;
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = athena::find_attachment(&dir, &target) else { return Ok(None) };
        let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !athena::ATTACHMENT_IMAGES.contains(&extension.as_str()) {
            return Ok(None);
        }
        if fs::metadata(&path).map(|m| m.len()).unwrap_or(u64::MAX) > athena::MAX_ATTACHMENT {
            return Err("Imagem grande demais para mostrar".to_string());
        }
        let bytes = fs::read(&path).map_err(|_| "Não foi possível ler o anexo".to_string())?;
        let mime = match extension.as_str() { "jpg" | "jpeg" | "jfif" => "image/jpeg", "svg" => "image/svg+xml", "gif" => "image/gif", "webp" => "image/webp", "bmp" => "image/bmp", "avif" => "image/avif", _ => "image/png" };
        Ok(Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))))
    }).await.map_err(|_| "Falha ao ler o anexo".to_string())?
}

/// Notas da mesma raiz que têm link `[[…]]` para esta (como os backlinks do Obsidian).
#[tauri::command]
pub async fn athena_backlinks(state: tauri::State<'_, Core>, root: String, path: String) -> Result<Vec<Value>, String> {
    let dir = state.athena_path(&state.prefs()?, &root)?;
    let key = state.sync_notes(&dir).await?;
    let rows = state.store.lock().map_err(|_| "Dados ocupados")?.notes_with_links(&key).map_err(|_| "Falha ao consultar notas")?;
    let mut out: Vec<Value> = Vec::new();
    for (note, text) in rows {
        let relative = note.trim_start_matches(&key).to_string();
        if relative == path || out.iter().any(|v| v["path"] == relative) {
            continue;
        }
        if athena::wiki_targets(&text).iter().any(|t| athena::links_to(t, &path)) {
            let title = relative.rsplit('/').next().unwrap_or(&relative).trim_end_matches(".md").to_string();
            out.push(json!({"path": relative, "title": title}));
        }
    }
    Ok(out)
}

/// Busca por palavras em todas as raízes: raiz, caminho, título e trecho com destaque.
#[tauri::command]
pub async fn athena_search(state: tauri::State<'_, Core>, query: String) -> Result<Vec<Value>, String> {
    let prefs = state.prefs()?;
    let lower = query.trim().to_lowercase();
    let fts = memory::fts_query(&query).or_else(|| (lower.chars().count() >= 2).then(|| format!("\"{}\"*", lower.replace('"', ""))));
    let mut out: Vec<Value> = Vec::new();
    for root in state.athena_roots(&prefs).into_iter().filter(|r| r.path.is_dir()) {
        let key = state.sync_notes(&root.path).await?;
        let dir = root.path.clone();
        let notes = tauri::async_runtime::spawn_blocking(move || athena::list(&dir)).await.unwrap_or_default();
        for note in notes.iter().filter(|n| !lower.is_empty() && n.title.to_lowercase().contains(&lower)) {
            out.push(json!({"root": root.id, "rootName": root.name, "path": note.path, "title": note.title, "snippet": ""}));
        }
        if let Some(fts) = &fts {
            for (note, snippet) in state.store.lock().map_err(|_| "Dados ocupados")?.search_notes(fts, &key, 20).unwrap_or_default() {
                let path = note.trim_start_matches(&key).to_string();
                if out.iter().any(|v| v["root"] == root.id && v["path"] == path) {
                    continue;
                }
                let title = path.rsplit('/').next().unwrap_or(&path).trim_end_matches(".md").to_string();
                out.push(json!({"root": root.id, "rootName": root.name, "path": path, "title": title, "snippet": snippet}));
            }
        }
    }
    Ok(out)
}

/// Última sincronização com Git por raiz (segundos Unix e resultado), mostrada na Athena.
static ATHENA_SYNC: Mutex<Option<HashMap<String, (i64, athena_git::Outcome)>>> = Mutex::new(None);
fn last_sync(root: &str) -> Option<(i64, athena_git::Outcome)> {
    ATHENA_SYNC.lock().ok().and_then(|m| m.as_ref().and_then(|m| m.get(root).cloned()))
}
fn record_sync(root: &str, outcome: athena_git::Outcome) {
    if let Ok(mut map) = ATHENA_SYNC.lock() {
        map.get_or_insert_with(HashMap::new).insert(root.to_string(), (store::now(), outcome));
    }
}

#[tauri::command]
pub async fn athena_git_status(state: tauri::State<'_, Core>, root: String) -> Result<Value, String> {
    let prefs = state.prefs()?;
    let dir = state.athena_path(&prefs, &root)?;
    let status = tauri::async_runtime::spawn_blocking(move || athena_git::status(&dir)).await.map_err(|_| "Falha ao consultar o Git")?;
    let last = last_sync(&root);
    Ok(json!({"enabled": prefs.athena_git, "status": status, "obsidian": athena_git::obsidian_running(), "last": last.map(|(at, outcome)| json!({"at": at, "outcome": outcome}))}))
}

/// Sincronizar agora (commit, puxar e enviar, conforme o obsidian-git do cofre).
#[tauri::command]
pub async fn athena_git_sync(state: tauri::State<'_, Core>, root: String) -> Result<athena_git::Outcome, String> {
    let dir = state.athena_path(&state.prefs()?, &root)?;
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let settings = athena_git::settings(&dir);
        athena_git::sync(&dir, &settings, reminders::now_local())
    }).await.map_err(|_| "Falha ao sincronizar")??;
    record_sync(&root, outcome.clone());
    Ok(outcome)
}

/// Sincronização automática no intervalo do obsidian-git (0: só manual) e "puxar ao abrir", para
/// cada raiz que for repositório. Não roda com o Obsidian aberto: lá o próprio plugin cuida disso.
pub async fn athena_git_tick(app: &tauri::AppHandle, boot: bool) {
    use tauri::{Emitter, Manager};
    let Some(state) = app.try_state::<Core>() else { return };
    let Ok(prefs) = state.prefs() else { return };
    if !prefs.athena_git || athena_git::obsidian_running() {
        return;
    }
    for root in state.athena_roots(&prefs) {
        if !athena_git::is_repo(&root.path) {
            continue;
        }
        let settings = athena_git::settings(&root.path);
        if boot && settings.pull_on_open {
            let (dir, s) = (root.path.clone(), settings.clone());
            let _ = tauri::async_runtime::spawn_blocking(move || athena_git::pull(&dir, &s)).await;
            let _ = app.emit("notes-updated", json!({}));
        }
        if settings.interval == 0 {
            continue;
        }
        match last_sync(&root.id) {
            None => record_sync(&root.id, athena_git::Outcome { message: "Aguardando o intervalo do obsidian-git.".into(), ..Default::default() }),
            Some((at, _)) if store::now() - at >= settings.interval as i64 * 60 => {
                let dir = root.path.clone();
                if let Ok(Ok(outcome)) = tauri::async_runtime::spawn_blocking(move || athena_git::sync(&dir, &settings, reminders::now_local())).await {
                    record_sync(&root.id, outcome);
                    let _ = app.emit("notes-updated", json!({}));
                }
            }
            _ => {}
        }
    }
}

/// Apaga uma pasta da raiz: vai inteira para a lixeira (`.trash`), nada some de vez.
#[tauri::command]
pub fn athena_trash_folder(state: tauri::State<'_, Core>, root: String, path: String) -> Result<(), String> {
    let dir = state.athena_path(&state.prefs()?, &root)?;
    trash_folder_in(&dir, &path)
}

/// Apagar pasta: uma pasta protegida só sai (inteira ou em parte) aberta com a senha; uma pasta
/// comum com uma protegida dentro não sai.
fn trash_folder_in(dir: &std::path::Path, path: &str) -> Result<(), String> {
    if let Some(place) = protected(&dir, &path) {
        let key = unlocked(&place)?;
        if place.inner.is_empty() {
            // A pasta inteira vai para a lixeira ainda cifrada, com a proteção junto.
            athena::trash_folder(dir, path)?;
            athena_lock::end_session(&place.dir);
            return Ok(());
        }
        return athena_lock::trash(&place.dir, &key, &place.inner, &dir.join(".trash"));
    }
    if !athena_lock::protected_folders(&dir.join(path)).is_empty() {
        return Err("Esta pasta tem uma pasta protegida dentro; apague a protegida primeiro".into());
    }
    athena::trash_folder(dir, path)
}

/// Primeira pasta protegida e bloqueada dentro da pasta, se houver.
fn locked_inside(dir: &std::path::Path) -> Option<String> {
    athena_lock::protected_folders(dir).into_iter().find(|folder| athena_lock::session(&dir.join(folder)).is_none())
}

// ---- Pastas protegidas da Athena ----

/// Caminho dentro de uma pasta protegida: a pasta (no disco e relativa à raiz) e o caminho interno.
struct Protected {
    dir: PathBuf,
    folder: String,
    inner: String,
}

fn protected(root: &std::path::Path, path: &str) -> Option<Protected> {
    let folder = athena_lock::protected_folder_of(root, path)?;
    let inner = path.trim_matches('/').strip_prefix(&folder).unwrap_or("").trim_start_matches('/').to_string();
    Some(Protected { dir: root.join(&folder), folder, inner })
}

fn unlocked(place: &Protected) -> Result<athena_lock::Key, String> {
    athena_lock::session(&place.dir).map(|(key, _)| key).ok_or_else(|| "Pasta protegida: digite a senha na Athena para abri-la".into())
}

fn protected_note(inner: &str) -> Result<(), String> {
    if inner.to_lowercase().ends_with(".md") { Ok(()) } else { Err("Nota inválida".into()) }
}

/// Imagem colada numa nota protegida: fica cifrada na mesma pasta, em `Imagens/<nota>/`.
fn protected_paste(place: &Protected, before: &str, after: &str, expected: i64, bytes: &[u8]) -> Result<(String, i64), String> {
    let key = unlocked(place)?;
    protected_note(&place.inner)?;
    if bytes.len() as u64 > athena::MAX_ATTACHMENT {
        return Err("A imagem passa de 15 MB".into());
    }
    let extension = athena::image_extension(bytes).ok_or("Formato de imagem incompatível")?;
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|_| "Não foi possível gerar o nome da imagem")?;
    let name = format!("Imagem-{}.{extension}", random.iter().map(|b| format!("{b:02x}")).collect::<String>());
    let title = std::path::Path::new(&place.inner).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let image = format!("Imagens/{}/{name}", athena::file_name(&title));
    athena_lock::write(&place.dir, &key, &image, bytes, None)?;
    // Links pelo caminho a partir da raiz; títulos com sintaxe wiki usam o nome único.
    let target = if title.contains(['[', ']', '#', '|']) { name } else { format!("{}/{image}", place.folder) };
    let content = format!("{before}![[{target}]]{after}");
    match athena_lock::write(&place.dir, &key, &place.inner, content.as_bytes(), Some(expected)) {
        Ok(entry) => Ok((target, entry.modified)),
        Err(error) => {
            let _ = athena_lock::remove(&place.dir, &key, &image);
            Err(error)
        }
    }
}

/// Banner de uma nota protegida: só imagens da mesma pasta protegida.
fn protected_banner(root: &std::path::Path, place: &Protected, content: &str, expected: i64, image: &str) -> Result<(String, i64), String> {
    let key = unlocked(place)?;
    protected_note(&place.inner)?;
    let target = protected(root, image).filter(|p| p.dir == place.dir).ok_or("Escolha uma imagem desta mesma pasta protegida")?;
    let is_image = athena::attachment_mime(std::path::Path::new(&target.inner)).is_some_and(|m| m.starts_with("image/"));
    if !is_image || !athena_lock::entries(&place.dir, &key).iter().any(|e| e.path == target.inner && e.kind == "file") {
        return Err("Escolha uma imagem desta mesma pasta protegida".into());
    }
    let updated = athena::with_banner(content, image.trim_matches('/'));
    let entry = athena_lock::write(&place.dir, &key, &place.inner, updated.as_bytes(), Some(expected))?;
    Ok((updated, entry.modified))
}

/// Move uma nota ou anexo entre lugares comuns e pastas protegidas (ou dentro de uma). O destino é
/// gravado antes de a origem sair; nada fica decifrado no disco a não ser quando o destino é comum.
fn move_across(root: &std::path::Path, from: &str, to: &str) -> Result<String, String> {
    let to = to.trim_matches('/').to_string();
    let (source, target) = (protected(root, from), protected(root, &to));
    if let (Some(a), Some(b)) = (&source, &target) {
        if a.dir == b.dir {
            let key = unlocked(a)?;
            athena_lock::rename(&a.dir, &key, &a.inner, &b.inner)?;
            return Ok(to);
        }
    }
    let bytes = match &source {
        Some(place) => athena_lock::read(&place.dir, &unlocked(place)?, &place.inner)?.0,
        None => {
            let path = athena::safe_folder(from).filter(|p| !p.as_os_str().is_empty()).ok_or("Arquivo inválido")?;
            fs::read(root.join(path)).map_err(|_| "Arquivo não encontrado")?
        }
    };
    match &target {
        Some(place) => {
            let key = unlocked(place)?;
            if place.inner.is_empty() || athena_lock::entries(&place.dir, &key).iter().any(|e| e.path == place.inner) {
                return Err("Já existe um arquivo com esse nome nessa pasta".into());
            }
            athena_lock::write(&place.dir, &key, &place.inner, &bytes, None)?;
        }
        None => {
            let path = athena::safe_folder(&to).filter(|p| !p.as_os_str().is_empty()).ok_or("Destino inválido")?;
            let path = root.join(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|_| "Não foi possível criar a pasta")?;
            }
            let mut file = fs::File::create_new(&path).map_err(|_| "Já existe um arquivo com esse nome nessa pasta")?;
            use std::io::Write;
            file.write_all(&bytes).and_then(|_| file.sync_all()).map_err(|_| "Não foi possível gravar o arquivo")?;
        }
    }
    match &source {
        Some(place) => athena_lock::remove(&place.dir, &unlocked(place)?, &place.inner)?,
        None => fs::remove_file(root.join(from)).map_err(|_| "Não foi possível tirar o arquivo da origem")?,
    }
    Ok(to)
}

/// Mídia de uma pasta protegida desbloqueada (pelo caminho ou, como nos links do Obsidian, pelo
/// nome). `None`: não é de pasta protegida.
fn protected_media(state: &Core, root: &str, target: &str) -> Option<Result<(Vec<u8>, &'static str), String>> {
    let dir = state.athena_path(&state.prefs().ok()?, root).ok()?;
    let target = target.split('#').next().unwrap_or(target).trim();
    let found = if let Some(place) = protected(&dir, target) {
        Some(place)
    } else if athena::find_attachment(&dir, target).is_some() {
        None
    } else {
        let name = target.rsplit('/').next().unwrap_or(target).to_lowercase();
        athena_lock::protected_folders(&dir).into_iter().find_map(|folder| {
            let place = dir.join(&folder);
            let (key, _) = athena_lock::session(&place)?;
            let entry = athena_lock::entries(&place, &key).into_iter()
                .find(|e| e.kind == "file" && e.path.rsplit('/').next().unwrap_or(&e.path).to_lowercase() == name)?;
            Some(Protected { dir: place, folder: folder.clone(), inner: entry.path })
        })
    }?;
    Some((|| {
        let mime = athena::attachment_mime(std::path::Path::new(&found.inner)).ok_or("Tipo de arquivo não mostrado")?;
        let key = unlocked(&found)?;
        Ok((athena_lock::read(&found.dir, &key, &found.inner)?.0, mime))
    })())
}

/// Protege uma pasta com senha (os arquivos passam a ficar cifrados no disco e no Git). Devolve a
/// chave de recuperação, que só existe nesta resposta.
#[tauri::command]
pub async fn athena_protect(app: tauri::AppHandle, state: tauri::State<'_, Core>, root: String, folder: String, password: String) -> Result<Value, String> {
    use tauri::Emitter;
    let dir = state.athena_path(&state.prefs()?, &root)?;
    let (count, recovery) = tauri::async_runtime::spawn_blocking(move || athena_lock::protect(&dir, &folder, &password))
        .await.map_err(|_| "Falha ao proteger a pasta")??;
    let _ = app.emit("notes-updated", json!({"root": root}));
    Ok(json!({"count": count, "recovery": recovery}))
}

/// Senha esquecida: a chave de recuperação define uma senha nova e já abre a pasta.
#[tauri::command]
pub async fn athena_recover(state: tauri::State<'_, Core>, root: String, folder: String, code: String, password: String) -> Result<i64, String> {
    let dir = state.athena_path(&state.prefs()?, &root)?;
    let place = protected(&dir, &folder).filter(|p| p.inner.is_empty()).ok_or("Esta pasta não está protegida")?;
    let (dir, key) = tauri::async_runtime::spawn_blocking(move || athena_lock::recover(&place.dir, &code, &password).map(|key| (place.dir, key)))
        .await.map_err(|_| "Falha ao recuperar a pasta")??;
    Ok(athena_lock::start_session(&dir, key))
}

/// Chave de recuperação nova (a anterior deixa de valer), com a senha atual.
#[tauri::command]
pub async fn athena_new_recovery(state: tauri::State<'_, Core>, root: String, folder: String, password: String) -> Result<String, String> {
    let dir = state.athena_path(&state.prefs()?, &root)?;
    let place = protected(&dir, &folder).filter(|p| p.inner.is_empty()).ok_or("Esta pasta não está protegida")?;
    tauri::async_runtime::spawn_blocking(move || athena_lock::new_recovery(&place.dir, &password))
        .await.map_err(|_| "Falha ao criar a chave de recuperação")?
}

/// Abre uma pasta protegida nesta execução da Ayla. Devolve o momento da senha (segundos Unix).
#[tauri::command]
pub async fn athena_unlock(state: tauri::State<'_, Core>, root: String, folder: String, password: String) -> Result<i64, String> {
    let dir = state.athena_path(&state.prefs()?, &root)?;
    let place = protected(&dir, &folder).filter(|p| p.inner.is_empty()).ok_or("Esta pasta não está protegida")?;
    let key = tauri::async_runtime::spawn_blocking(move || athena_lock::unlock(&place.dir, &password).map(|key| (place.dir, key)))
        .await.map_err(|_| "Falha ao conferir a senha")??;
    Ok(athena_lock::start_session(&key.0, key.1))
}

/// Volta a bloquear a pasta (a chave sai da memória).
#[tauri::command]
pub fn athena_lock(state: tauri::State<'_, Core>, root: String, folder: String) -> Result<(), String> {
    let dir = state.athena_path(&state.prefs()?, &root)?;
    athena_lock::end_session(&dir.join(folder.trim_matches('/')));
    Ok(())
}

/// Tira a proteção (com a senha): os arquivos voltam a ser comuns.
#[tauri::command]
pub async fn athena_unprotect(app: tauri::AppHandle, state: tauri::State<'_, Core>, root: String, folder: String, password: String) -> Result<usize, String> {
    use tauri::Emitter;
    let dir = state.athena_path(&state.prefs()?, &root)?;
    let place = protected(&dir, &folder).filter(|p| p.inner.is_empty()).ok_or("Esta pasta não está protegida")?;
    let count = tauri::async_runtime::spawn_blocking(move || {
        let key = athena_lock::unlock(&place.dir, &password)?;
        let count = athena_lock::unprotect(&place.dir, &key)?;
        athena_lock::end_session(&place.dir);
        Ok::<_, String>(count)
    }).await.map_err(|_| "Falha ao tirar a proteção")??;
    let _ = app.emit("notes-updated", json!({"root": root}));
    Ok(count)
}

/// Troca a senha de uma pasta protegida.
#[tauri::command]
pub async fn athena_change_password(state: tauri::State<'_, Core>, root: String, folder: String, current: String, password: String) -> Result<(), String> {
    let dir = state.athena_path(&state.prefs()?, &root)?;
    let place = protected(&dir, &folder).filter(|p| p.inner.is_empty()).ok_or("Esta pasta não está protegida")?;
    tauri::async_runtime::spawn_blocking(move || athena_lock::change_password(&place.dir, &current, &password))
        .await.map_err(|_| "Falha ao trocar a senha")?
}

/// Anexo (imagem, vídeo ou PDF) de uma raiz pelo nome ou caminho, como nos links do Obsidian.
fn athena_attachment_path(state: &Core, root: &str, target: &str) -> Result<PathBuf, String> {
    let dir = state.athena_path(&state.prefs()?, root)?;
    let path = athena::find_attachment(&dir, target).ok_or("Anexo não encontrado")?;
    athena::attachment_mime(&path).ok_or("Este tipo de arquivo não é mostrado na Athena")?;
    Ok(path)
}

/// Identifica o vínculo também nos endereços que já estão abertos quando a pasta muda.
fn iris_root_id(dir: &std::path::Path) -> String {
    use sha2::{Digest, Sha256};
    format!("iris:{:x}", Sha256::digest(dir.as_os_str().as_encoded_bytes()))
}

fn has_hidden_component(path: &std::path::Path) -> bool {
    use std::path::Component;
    path.components().any(|component| matches!(component, Component::Normal(name) if name.as_encoded_bytes().starts_with(b".")))
}

fn linked_media_path(dir: &std::path::Path, requested_root: &str, target: &str) -> Option<PathBuf> {
    use std::path::{Component, Path};
    if iris_root_id(dir) != requested_root || has_hidden_component(dir)
        || has_hidden_component(Path::new(target))
        || !Path::new(target).components().all(|p| matches!(p, Component::Normal(_))) {
        return None;
    }
    let path = dir.join(target);
    if path.canonicalize().ok()? != path || !path.is_file() {
        return None;
    }
    athena::attachment_mime(&path)?;
    Some(path)
}

fn readable_media_path(state: &Core, root: &str, target: &str) -> Result<PathBuf, String> {
    if root.starts_with("iris:") {
        return state.prefs()?.system_folders().iter().find_map(|dir| linked_media_path(dir, root, target))
            .ok_or_else(|| "Arquivo indisponível na pasta conectada".into());
    }
    athena_attachment_path(state, root, target)
}

#[tauri::command]
pub async fn iris_change_folder(app: tauri::AppHandle, state: tauri::State<'_, Core>, disconnect: bool, root: Option<String>) -> Result<(), String> {
    let folder = if disconnect { None } else {
        let Some(chosen) = athena_pick_folder(app).await? else { return Ok(()) };
        if has_hidden_component(std::path::Path::new(&chosen)) {
            return Err("A Iris ignora pastas ocultas".into());
        }
        let dir = fs::canonicalize(chosen).map_err(|_| "Pasta indisponível")?;
        if has_hidden_component(&dir) {
            return Err("A Iris ignora pastas ocultas".into());
        }
        fs::read_dir(&dir).map_err(|_| "Não foi possível ler a pasta")?;
        Some(dir)
    };
    let store = state.preferences.lock().map_err(|_| "Configuração ocupada")?;
    let mut prefs = store.load().map_err(|_| "Não foi possível ler a configuração")?;
    let mut folders = prefs.system_folders();
    if let Some(root) = root {
        let index = folders.iter().position(|dir| iris_root_id(dir) == root).ok_or("Pasta já desconectada")?;
        folders.remove(index);
        if let Some(folder) = folder {
            if !folders.contains(&folder) { folders.insert(index, folder); }
        }
    } else if let Some(folder) = folder {
        if !folders.contains(&folder) { folders.push(folder); }
    } else {
        return Err("Selecione a pasta para desconectar".into());
    }
    prefs.iris_folder = None;
    prefs.iris_folders = folders;
    store.save(&prefs).map_err(|_| "Não foi possível salvar a pasta".into())
}

#[tauri::command]
pub fn iris_move_folder(state: tauri::State<'_, Core>, root: String, target: String, after: bool) -> Result<(), String> {
    if root == target {
        return Ok(());
    }
    let store = state.preferences.lock().map_err(|_| "Configuração ocupada")?;
    let mut prefs = store.load().map_err(|_| "Não foi possível ler a configuração")?;
    let mut folders = prefs.system_folders();
    let index = folders.iter().position(|dir| iris_root_id(dir) == root).ok_or("Pasta já desconectada")?;
    let folder = folders.remove(index);
    let destination = folders.iter().position(|dir| iris_root_id(dir) == target).ok_or("Pasta de destino já desconectada")?;
    folders.insert(destination + usize::from(after), folder);
    prefs.iris_folder = None;
    prefs.iris_folders = folders;
    store.save(&prefs).map_err(|_| "Não foi possível salvar a ordem das pastas".into())
}

/// Miniaturas da Athena e da Iris, guardadas nos dados da Ayla (só neste computador).
fn thumbnails(state: &Core) -> PathBuf {
    state.root.join("cache").join("miniaturas")
}

/// Entrega o arquivo (ou a miniatura, com `?miniatura`, se for imagem), aceitando trechos.
fn media_response(state: &Core, request: &tauri::http::Request<Vec<u8>>, path: &std::path::Path, mime: &str) -> tauri::http::Response<Vec<u8>> {
    let range = request.headers().get(tauri::http::header::RANGE).and_then(|v| v.to_str().ok());
    let wants_thumb = request.uri().query().is_some_and(|q| q.split('&').any(|p| p == "miniatura"));
    if wants_thumb && mime.starts_with("video/") {
        let binary = state.runtime.tool("ffmpeg", "ffmpeg").unwrap_or_else(|| PathBuf::from("ffmpeg"));
        return media::video_thumbnail(&thumbnails(state), path, &binary)
            .map(|thumb| media::serve(&thumb, "image/jpeg", range)).unwrap_or_else(not_found);
    }
    if wants_thumb && mime.starts_with("image/") {
        return media::thumbnail(&thumbnails(state), path, false)
            .map(|thumb| media::serve(&thumb, "image/jpeg", range)).unwrap_or_else(not_found);
    }
    // Prévia do PDF (primeira página); sem ela, a interface mostra o ícone, sem baixar o PDF inteiro.
    if wants_thumb && mime == "application/pdf" {
        return media::thumbnail(&thumbnails(state), path, true).map(|thumb| media::serve(&thumb, "image/jpeg", range)).unwrap_or_else(not_found);
    }
    media::serve(path, mime, range)
}

fn not_found() -> tauri::http::Response<Vec<u8>> {
    tauri::http::Response::builder().status(tauri::http::StatusCode::NOT_FOUND).body(Vec::new()).unwrap_or_default()
}

/// `athena-file://localhost/<raiz>/<anexo>`: imagens, vídeos e PDFs das notas, lidos direto da pasta.
pub fn athena_file(state: &Core, request: &tauri::http::Request<Vec<u8>>) -> tauri::http::Response<Vec<u8>> {
    let trimmed = request.uri().path().trim_start_matches('/');
    let Some((root, target)) = trimmed.split_once('/') else { return not_found() };
    let (Some(root), Some(target)) = (athena::percent_decode(root), athena::percent_decode(target)) else { return not_found() };
    if !root.starts_with("iris:") {
        match protected_media(state, &root, &target) {
            Some(Ok((bytes, mime))) => {
                // Sem miniaturas (seriam cópias no disco): imagens vão inteiras; vídeo e PDF mostram o ícone.
                let thumb = request.uri().query().is_some_and(|q| q.split('&').any(|p| p == "miniatura"));
                if thumb && !mime.starts_with("image/") {
                    return not_found();
                }
                let range = request.headers().get(tauri::http::header::RANGE).and_then(|v| v.to_str().ok());
                return media::serve_bytes(&bytes, mime, range);
            }
            Some(Err(_)) => return not_found(),
            None => {}
        }
    }
    let Ok(path) = readable_media_path(state, &root, &target) else { return not_found() };
    let mime = athena::attachment_mime(&path).unwrap_or("application/octet-stream");
    media_response(state, request, &path, mime)
}

/// Tipo de um arquivo guardado (anexo ou imagem gerada) para mostrar na Iris.
fn stored_mime(file: &FileInfo) -> Option<&'static str> {
    match file.kind {
        Kind::Image | Kind::Gif | Kind::Video | Kind::Pdf => athena::attachment_mime(std::path::Path::new(&file.name)).or(match file.kind {
            Kind::Gif => Some("image/gif"),
            Kind::Pdf => Some("application/pdf"),
            Kind::Video => Some("video/mp4"),
            _ => Some("image/png"),
        }),
        _ => None,
    }
}

/// `iris-file://localhost/<id>`: imagem, vídeo ou PDF guardado nos dados da Ayla.
pub fn iris_file(state: &Core, request: &tauri::http::Request<Vec<u8>>) -> tauri::http::Response<Vec<u8>> {
    let Some(id) = athena::percent_decode(request.uri().path().trim_start_matches('/')) else { return not_found() };
    let Ok(Some(file)) = state.store.lock().map(|db| db.file(&id).ok().flatten()) else { return not_found() };
    let Some(mime) = stored_mime(&file) else { return not_found() };
    media_response(state, request, &state.root.join(&file.path), mime)
}

/// Iris: todas as imagens, vídeos e PDFs — das raízes da Athena (cofres, notas, projetos), os
/// anexos enviados no chat, as imagens que o chat gerou e os arquivos adicionados direto —,
/// do mais recente ao mais antigo, com o tipo e a origem de cada um.
#[tauri::command]
pub async fn iris_library(state: tauri::State<'_, Core>) -> Result<Value, String> {
    iris_library_for(&state).await
}

async fn iris_library_for(state: &Core) -> Result<Value, String> {
    let prefs = state.prefs()?;
    let mut roots: Vec<AthenaRoot> = state.athena_roots(&prefs).into_iter().filter(|r| r.available).collect();
    for dir in prefs.system_folders() {
        if has_hidden_component(&dir) {
            continue;
        }
        let available = dir.canonicalize().ok().as_ref() == Some(&dir) && fs::read_dir(&dir).is_ok();
        roots.push(AthenaRoot { id: iris_root_id(&dir), kind: "computer", available,
            name: dir.file_name().unwrap_or(dir.as_os_str()).to_string_lossy().into_owned(), path: dir });
    }
    let mut items = state.iris_stored()?;
    let projects_with_media: std::collections::HashSet<String> = items.iter()
        .filter_map(|(_, item)| item["root"].as_str().map(str::to_owned))
        .collect();
    let (scanned, folders, athena_roots) = tauri::async_runtime::spawn_blocking(move || {
        let mut folders = Vec::new();
        let mut athena_roots = Vec::new();
        let mut out = Vec::new();
        for root in roots {
            let computer = root.kind == "computer";
            let limit = (!computer).then_some(athena::MAX_NOTES);
            let (entries, dirs) = if root.available { athena::scan_attachments(&root.path, limit) } else { Default::default() };
            let has_media = !entries.is_empty() || projects_with_media.contains(&root.id);
            let tree = json!({"id": root.id, "path": root.path, "name": root.name, "available": root.available, "folders": dirs, "hasMedia": has_media});
            if computer {
                folders.push(tree);
            } else if has_media {
                athena_roots.push(tree);
            }
            for found in entries {
                let Some(mime) = athena::attachment_mime(std::path::Path::new(&found.path)) else { continue };
                let folder = found.path.rsplit_once('/').map(|(f, _)| f.to_string()).unwrap_or_default();
                let place = if folder.is_empty() { root.name.clone() } else { format!("{} / {folder}", root.name) };
                out.push((found.modified, json!({
                    "key": format!("{}|{}", root.id, found.path), "kind": kind_of_mime(mime), "name": found.title, "source": if computer { "computer" } else { "athena" }, "place": place,
                    "date": found.modified, "bytes": found.bytes, "root": root.id, "path": found.path,
                })));
            }
        }
        (out, folders, athena_roots)
    }).await.map_err(|_| "Falha ao ler os cofres")?;
    items.extend(scanned);
    items.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(json!({"items": items.into_iter().map(|(_, item)| item).collect::<Vec<_>>(), "roots": folders, "athenaRoots": athena_roots}))
}

fn kind_of_mime(mime: &str) -> &'static str {
    if mime == "application/pdf" { "pdf" } else if mime.starts_with("video/") { "video" } else { "image" }
}

/// Move um anexo (imagem ou PDF) para outra pasta da mesma raiz.
#[tauri::command]
pub fn athena_move_file(state: tauri::State<'_, Core>, root: String, path: String, folder: String) -> Result<String, String> {
    let dir = state.athena_path(&state.prefs()?, &root)?;
    if protected(&dir, &path).is_some() || protected(&dir, &folder).is_some() {
        let name = path.rsplit('/').next().unwrap_or(&path).to_string();
        let to = if folder.is_empty() { name } else { format!("{folder}/{name}") };
        return move_across(&dir, &path, &to);
    }
    athena::move_attachment(&dir, &path, &folder)
}

/// Serviço único da Athena e da Iris para abrir uma mídia (imagem, vídeo ou PDF) no aplicativo
/// do sistema: um arquivo guardado na Ayla (`file`) ou um arquivo de uma raiz da Athena (`root` + `path`).
#[tauri::command]
pub fn media_open(state: tauri::State<'_, Core>, file: Option<String>, root: Option<String>, path: Option<String>) -> Result<(), String> {
    let target = match (file, root, path) {
        (Some(id), _, _) => {
            let file = state.store.lock().map_err(|_| "Dados ocupados")?.file(&id).ok().flatten().ok_or("Arquivo não encontrado")?;
            stored_mime(&file).ok_or("Este tipo de arquivo não é uma mídia da Ayla")?;
            state.root.join(&file.path)
        }
        (None, Some(root), Some(path)) => {
            if !root.starts_with("iris:") && protected(&state.athena_path(&state.prefs()?, &root)?, &path).is_some() {
                return Err("Arquivos de pastas protegidas só abrem na Athena".into());
            }
            readable_media_path(&state, &root, &path)?
        }
        _ => return Err("Arquivo não informado".into()),
    };
    media::open_in_system(&target)
}

fn trash_image(path: &std::path::Path) -> Result<(), String> {
    use webkit2gtk::gio::{self, prelude::FileExt};
    gio::File::for_path(path).trash(gio::Cancellable::NONE)
        .map_err(|_| "Não foi possível mover a imagem para a lixeira do sistema.".into())
}

#[tauri::command]
pub async fn iris_trash_image(app: tauri::AppHandle, file: Option<String>, root: Option<String>, path: Option<String>) -> Result<(), String> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Core>();
        match (file, root, path) {
            (Some(id), None, None) => {
                let db = state.store.lock().map_err(|_| "Dados ocupados")?;
                let file = db.file(&id).map_err(|_| "Falha ao localizar a imagem")?.ok_or("Imagem não encontrada")?;
                if !matches!(file.kind, Kind::Image | Kind::Gif) {
                    return Err("Selecione uma imagem".into());
                }
                trash_image(&state.root.join(file.path))?;
                db.delete_file(&id).map_err(|_| "A imagem foi para a lixeira, mas a Ayla não conseguiu atualizar o registro.".to_string())?;
                Ok(())
            }
            (None, Some(root), Some(path)) => {
                let target = if root.starts_with("iris:") {
                    readable_media_path(&state, &root, &path)?
                } else {
                    let dir = state.athena_path(&state.prefs()?, &root)?.canonicalize().map_err(|_| "Pasta indisponível")?;
                    let relative = athena::safe_folder(&path).ok_or("Caminho inválido")?;
                    let target = dir.join(relative);
                    if target.canonicalize().ok().as_ref() != Some(&target) || !target.is_file() {
                        return Err("Imagem não encontrada".into());
                    }
                    target
                };
                if !athena::attachment_mime(&target).is_some_and(|mime| mime.starts_with("image/")) {
                    return Err("Selecione uma imagem".into());
                }
                trash_image(&target)
            }
            _ => Err("Selecione uma imagem".into()),
        }
    }).await.map_err(|_| "Falha ao enviar a imagem para a lixeira")?
}

/// Abre a nota no Obsidian (onde os plugins do cofre rodam).
#[tauri::command]
pub fn athena_open_obsidian(state: tauri::State<'_, Core>, root: String, path: String) -> Result<(), String> {
    let dir = state.athena_path(&state.prefs()?, &root)?;
    if protected(&dir, &path).is_some() {
        return Err("O Obsidian não abre notas de pastas protegidas da Athena".into());
    }
    let relative = athena::safe_relative(&path).ok_or("Nota inválida")?;
    let link = athena_git::obsidian_link(&dir.join(relative));
    std::process::Command::new("xdg-open").arg(link).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().map(|_| ()).map_err(|_| "Não foi possível abrir o Obsidian".into())
}

#[tauri::command]
pub fn athena_open_folder(state: tauri::State<'_, Core>, root: String) -> Result<(), String> {
    let dir = state.athena_path(&state.prefs()?, &root)?;
    std::process::Command::new("xdg-open").arg(&dir).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().map(|_| ()).map_err(|_| "Não foi possível abrir a pasta".into())
}

/// Referências sugeridas para a revisão de envio à nuvem (memórias públicas relevantes).
#[tauri::command]
pub async fn cloud_references(state: tauri::State<'_, Core>, project: Option<String>, message: String) -> Result<Vec<memory::Reference>, String> {
    if message.trim().chars().count() < 3 || message.len() > 200_000 {
        return Ok(Vec::new());
    }
    let prefs = state.prefs()?;
    let facts = state.references(&prefs, project.as_deref(), &message, &CancellationToken::new()).await;
    Ok(facts.into_iter().map(|f| memory::Reference { id: f.id, content: f.content }).collect())
}

#[tauri::command]
pub async fn chat(
    app: tauri::AppHandle,
    state: tauri::State<'_, Core>,
    request: ChatRequest,
    events: Channel<Value>,
) -> Result<(), String> {
    if request.message.trim().is_empty() || request.message.len() > 200000 {
        return Err("Mensagem vazia ou muito longa".into());
    }
    if !matches!(
        request.capability,
        None | Some(Capability::Text | Capability::Reasoning | Capability::Vision)
    ) {
        return Err("Tipo de conversa inválido".into());
    }
    let token = state.job(&request.id)?;
    // Outra conversa respondendo: esta espera a vez (o modelo local atende um pedido por vez).
    let _gate = match state.take_gate(&token, || { let _ = events.send(json!({"type":"status","text":"Esperando a outra resposta terminar…"})); }).await {
        Ok(gate) => gate,
        Err(error) => { state.finish(&request.id); return Err(error); }
    };
    let start = Instant::now();
    let mut partial = String::new();
    let mut first = None;
    let mut chunks = 0;
    let mut metric_model = String::from("unresolved");
    let mut origin = None;
    let mut recalled = 0usize;
    let mut result=async{
        let prefs=state.prefs()?;
        // Lembretes: criados neste computador e confirmados com data e hora, sem passar pela nuvem.
        if reminders::is_request(&request.message)&&request.attachments.is_empty(){
            let now=reminders::now_local();
            let mut parsed=reminders::parse(&request.message,now);
            if parsed.is_none(){
                if let Ok(model)=state.models().and_then(|m|models::choose(&m,Capability::Text,state.budget(&prefs),prefs.context_tokens,None)){
                    let _=events.send(json!({"type":"status","text":"Entendendo o lembrete…"}));
                    parsed=state.runtime.complete_json(&model,&prefs,reminders::model_request(&request.message,now),reminders::model_schema(),&token).await.ok().and_then(|raw|reminders::parse_model(&raw,now));
                }
            }
            // Sem assunto no pedido ("configura o aviso daqui 10 segundos"): pela conversa recente.
            if let Some(p)=parsed.as_mut().filter(|p|reminders::generic(&p.text)){
                let recent=state.store.lock().map_err(|_|"Dados ocupados")?.context(&request.conversation,6,3000).unwrap_or_default().iter()
                    .filter_map(|m|Some(format!("{}: {}",if m["role"]=="user"{"Pessoa"}else{"Ayla"},m["content"].as_str()?.chars().take(500).collect::<String>()))).collect::<Vec<_>>().join("\n");
                if !recent.is_empty(){
                    if let Ok(model)=state.models().and_then(|m|models::choose(&m,Capability::Text,state.budget(&prefs),prefs.context_tokens,None)){
                        if let Some(subject)=state.runtime.complete_json(&model,&prefs,reminders::subject_request(&request.message,&recent),reminders::subject_schema(),&token).await.ok().and_then(|raw|reminders::parse_subject(&raw)){p.text=subject;}
                    }
                }
            }
            let reply=match parsed{
                Some(p)=>{
                    state.store.lock().map_err(|_|"Dados ocupados")?.add_reminder(&p.text,reminders::to_unix(p.when),p.repeat.as_str(),Some(&request.conversation)).map_err(|_|"Não foi possível guardar o lembrete")?;
                    use tauri::Emitter;let _=app.emit("reminders-updated",());
                    reminders::confirmation(&p,now)
                }
                None=>"Não entendi quando devo te lembrar. Diga, por exemplo: \"me lembra amanhã às 9h de ligar pro médico\" ou \"me lembra toda segunda às 7h de ir pra academia\".".to_string(),
            };
            metric_model="lembretes".into();first=Some(start.elapsed().as_millis() as u64);chunks+=1;partial.push_str(&reply);
            let _=events.send(json!({"type":"token","text":reply}));
            return Ok(());
        }
        let (mut context,consent)={let db=state.store.lock().map_err(|_|"Dados ocupados")?;(db.context(&request.conversation,prefs.history_messages,prefs.history_chars).map_err(|_|"Falha ao consultar conversa")?,db.consent(&request.conversation).map_err(|_|"Conversa não encontrada")?)};
        let referenced=request.references.iter().map(|r|r.content.as_str()).collect::<Vec<_>>().join("\n");
        // Athena: "cria uma nota…" e "salva isso como nota" gravam a nota neste computador.
        if athena::note_request(&request.message)&&request.attachments.is_empty(){
            let note_root=match state.store.lock().map_err(|_|"Dados ocupados")?.conversation_project(&request.conversation).ok().flatten(){Some(p)=>format!("project:{p}"),None=>"ayla".to_string()};
            let root=state.athena_path(&prefs,&note_root)?;
            let last_answer=context.iter().rev().find(|m|m["role"]=="assistant").and_then(|m|m["content"].as_str()).map(str::to_string);
            let mut note=if athena::save_last_request(&request.message){
                last_answer.map(|answer|(athena::file_name(&quick_title(answer.lines().find(|l|!l.trim().is_empty()).unwrap_or("Resposta da Ayla").trim_start_matches('#').trim())),answer))
            }else{athena::explicit_note(&request.message)};
            if note.is_none()&&!athena::save_last_request(&request.message){
                if let Ok(model)=state.models().and_then(|m|models::choose(&m,Capability::Text,state.budget(&prefs),prefs.context_tokens,None)){
                    let _=events.send(json!({"type":"status","text":"Escrevendo a nota…"}));
                    let recent=context.iter().rev().take(6).rev().filter_map(|m|Some(format!("{}: {}",if m["role"]=="user"{"Pessoa"}else{"Ayla"},m["content"].as_str()?.chars().take(1500).collect::<String>()))).collect::<Vec<_>>().join("\n");
                    note=state.runtime.complete_json(&model,&prefs,athena::model_request(&request.message,&recent),athena::model_schema(),&token).await.ok().and_then(|raw|athena::parse_model(&raw));
                }
            }
            let reply=match note{
                Some((title,content))=>{
                    let path=athena::create(&root,&title,&format!("{}\n",content.trim_end()))?;
                    use tauri::Emitter;let _=app.emit("notes-updated",json!({"root":note_root,"path":path}));
                    format!("Criei a nota **{}** na Athena.",path.trim_end_matches(".md"))
                }
                None if athena::save_last_request(&request.message)=>"Ainda não há uma resposta nesta conversa para salvar como nota.".to_string(),
                None=>{
                    let title=athena::title_from(&request.message);
                    let path=athena::create(&root,&title,&format!("# {title}\n\n"))?;
                    use tauri::Emitter;let _=app.emit("notes-updated",json!({"root":note_root,"path":path}));
                    format!("Criei a nota **{}** na Athena, ainda vazia. Abra a Athena na lateral para escrever, ou me diga o conteúdo depois de dois pontos (\"cria uma nota sobre X: …\").",path.trim_end_matches(".md"))
                }
            };
            metric_model="athena".into();first=Some(start.elapsed().as_millis() as u64);chunks+=1;partial.push_str(&reply);
            let _=events.send(json!({"type":"token","text":reply}));
            return Ok(());
        }
        use super::research::AutoRoute;
        let auto_route = if request.automatic && prefs.usage_mode != UsageMode::LocalOnly
            && (prefs.usage_mode != UsageMode::PerConversation || consent)
            && prefs.cloud_platform().is_some() && request.sensitivity != Sensitivity::Sensitive
            && request.attachments.is_empty() && request.references.is_empty() {
            match request.auto_route.as_deref() {
                Some(choice) => super::research::confirmed_route(&request.message, choice),
                None => super::research::automatic_route(&state,&prefs,&request.message,&token).await,
            }
        } else { AutoRoute::Local };
        // Sem a resposta da pessoa, a dúvida fica neste computador.
        let auto_route = if auto_route == AutoRoute::Ask { AutoRoute::Local } else { auto_route };
        if token.is_cancelled() { return Err("Resposta interrompida".into()); }
        let use_cloud = if request.automatic { auto_route != AutoRoute::Local }
            else { prefs.usage_mode==UsageMode::CloudOnly || request.request_cloud };
        let web_search = if request.automatic { auto_route == AutoRoute::Search } else { request.web_search };
        // Finanças: a Ayla consulta o resumo da Moneta. Esses dados ficam neste computador em
        // qualquer modo, então a resposta é local (com modelo, ou direta, sem modelo).
        let finance={use tauri::Manager;let organ=app.state::<super::organs::Organ>();
            if organ.installed()&&super::organs::finance_question(&request.message){Some(organ.finance().await)}else{None}};
        let use_cloud=use_cloud&&finance.is_none();
        if let Some(data)=&finance{let _=events.send(json!({"type":"routing","text":match data{Ok(_)=>"Moneta consultada · resposta neste computador".to_string(),Err(e)=>format!("Moneta indisponível: {e}")}}));}
        let public_search = super::research::suggests_web_search(&request.message);
        if !request.automatic && web_search && (!use_cloud || !public_search || !request.references.is_empty()) {
            return Err("O pedido de pesquisa mudou. Revise novamente o envio.".into());
        }
        if !request.automatic && !use_cloud && finance.is_none() && public_search && request.attachments.is_empty()
            && super::research::wants_youtube(&request.message) && request.sensitivity != Sensitivity::Sensitive {
            let reply = "Para pesquisar na internet e trazer fontes, escolha Automático ou Na nuvem nos ajustes desta resposta.";
            metric_model = "local".into();
            first = Some(start.elapsed().as_millis() as u64);
            partial.push_str(reply);
            chunks += 1;
            let _ = events.send(json!({"type":"token","text":reply}));
            return Ok(());
        }
        // Anexos ficam neste computador: lidos, transcritos ou convertidos em quadros localmente.
        if use_cloud && !request.attachments.is_empty(){return Err("Arquivos anexados ficam neste computador. Responda localmente para usá-los.".into());}
        let attached=state.attachments(&request)?;
        let mut texts=Vec::new();let mut images=Vec::new();
        for file in &attached{
            let prepared=state.prepare_file(&prefs,file,&token,&mut |event:Value|{let _=events.send(event);}).await?;
            if let Some(text)=prepared.text{texts.push((file.name.clone(),file.kind,text));}
            images.extend(prepared.images);
        }
        images.truncate(files::MAX_IMAGES);
        // Iris: comparar fotos no tempo. Só neste computador (fotos pessoais são sensíveis) e
        // quando a mensagem não trouxe imagens próprias.
        let mut photo_note=None;
        if !use_cloud && images.is_empty() && iris::comparison_question(&request.message){
            let photos=state.iris_photos(&request.conversation,&request.message,&request.attachments).await;
            if photos.len()>=2{
                let mut labels=Vec::new();let mut listed=Vec::new();
                for (file,date) in &photos{
                    let prepared=state.prepare_file(&prefs,file,&token,&mut |event:Value|{let _=events.send(event);}).await?;
                    if let Some(path)=prepared.images.into_iter().next(){images.push(path);labels.push((file.name.clone(),*date));listed.push(json!({"id":file.id,"name":file.name,"pages":[],"date":iris::date_label(*date)}));}
                }
                if labels.len()>=2{photo_note=Some(iris::photos_block(&labels));let _=events.send(json!({"type":"sources","files":listed}));}
                else{images.clear();}
            }
        }
        // Arquivos anteriores da conversa e os do projeto entram pela Iris (só os trechos relevantes).
        let lengths:Vec<usize>=texts.iter().map(|(_,_,t)|t.chars().count()).collect();
        let mut blocks:Vec<String>=texts.iter().zip(files::budgets(&lengths,files::PROMPT_BUDGET)).filter(|(_,limit)|*limit>0).map(|((name,kind,text),limit)|files::prompt_block(name,*kind,text,limit)).collect();
        if let Some(note)=photo_note{blocks.push(note);}
        let models=state.models()?;let mut kind=if !images.is_empty(){Capability::Vision}else{request.capability.unwrap_or_else(||models::request_kind(&request.message,false))};
        if request.capability.is_none() && kind==Capability::Reasoning && !models.iter().any(|m|m.supports(kind)){kind=Capability::Text;}
        if images.is_empty(){if let Some(selected)=request.model.as_ref().and_then(|id|models.iter().find(|m|m.id==*id)){
            if !selected.supports(kind)&&matches!(selected.capability,Capability::Text|Capability::Reasoning|Capability::Vision){kind=selected.capability;}
        }}
        let mut selection=models::choose(&models,kind,state.budget(&prefs),prefs.context_tokens,request.model.as_deref());
        if selection.is_err() && request.capability.is_none() && request.model.is_none() && kind==Capability::Reasoning {
            kind=Capability::Text;selection=models::choose(&models,kind,state.budget(&prefs),prefs.context_tokens,None);
        }
        let text=if blocks.is_empty(){request.message.clone()}else{format!("{}\n\n{}",request.message,blocks.join("\n\n"))};
        let mut content=json!(text);
        if !images.is_empty(){
            let mut parts=vec![json!({"type":"text","text":text})];
            for path in &images{
                let bytes=fs::read(path).map_err(|_|"Não foi possível ler imagem")?;
                let mime=if bytes.starts_with(b"\x89PNG"){"image/png"}else if bytes.starts_with(&[0xff,0xd8]){"image/jpeg"}else{return Err("Formato de imagem inesperado".into());};
                parts.push(json!({"type":"image_url","image_url":{"url":format!("data:{mime};base64,{}",base64::engine::general_purpose::STANDARD.encode(bytes))}}));
            }
            content=json!(parts);
        }
        // Memórias e o contexto do projeto só entram em respostas locais nesta etapa.
        let mut system=prefs.system_prompt().to_string();
        system.push_str(OPTIONS_GUIDE);
        if !use_cloud{
            system.push_str("\n\nVocê é o aplicativo Ayla, neste computador. Você cria lembretes com notificação do sistema quando a pessoa pede (\"me lembra daqui 10 minutos de beber água\", \"me avisa amanhã às 9h\") e notas na Athena (\"cria uma nota sobre…\", \"salva isso como nota\"). Nunca diga que não consegue criar lembretes ou notificações; se faltar o horário ou o assunto, peça o que falta nesse formato.");
            let project = {
                let db = state.store.lock().map_err(|_| "Dados ocupados")?;
                db.conversation_project(&request.conversation).ok().flatten()
                    .and_then(|id| db.project(&id).ok().flatten())
            };
            if let Some(p)=&project{system.push_str(&project_block(p));}
            let facts=state.recall(&prefs,&request.message,project.as_ref().map(|p|p.id.as_str()),&token).await;recalled=facts.len();if let Some(block)=memory::context_block(&facts){system.push_str(&block);}
            let found=state.iris(&prefs,&request.conversation,&request.message,&request.attachments,&token).await;
            if let Some(block)=iris::context_block(&found){system.push_str(&block);let _=events.send(json!({"type":"sources","files":iris::sources(&found)}));}
            let notes=state.notes_context(&prefs,&request.message,project.as_ref().map(|p|p.id.as_str()),&token).await;
            if let Some(block)=athena::context_block(&notes){
                system.push_str(&block);
                let mut listed:Vec<Value>=Vec::new();
                for (title,_) in &notes{if !listed.iter().any(|v|v["name"]==format!("{title} (nota)")){listed.push(json!({"id":format!("nota:{title}"),"name":format!("{title} (nota)"),"pages":[]}));}}
                let _=events.send(json!({"type":"sources","files":listed}));
            }
            match &finance{
                Some(Ok(resumo))=>system.push_str(&format!("\n\n{}\nPara perguntas sobre finanças, use só estes dados da Moneta e cite os valores exatos. Se algo não estiver aqui, diga que a Moneta não informa; não invente números.",super::organs::finance_text(resumo))),
                Some(Err(error))=>system.push_str(&format!("\n\nA Moneta (finanças) não pôde ser consultada agora: {error} Se a pessoa perguntar sobre finanças, explique isso e não invente valores.")),
                None=>{}
            }
        }
        context.insert(0,json!({"role":"system","content":system}));
        context.push(json!({"role":"user","content":content}));
        let mut report=|event:Value|{if let Some(text)=event["text"].as_str(){if first.is_none(){first=Some(start.elapsed().as_millis() as u64);}partial.push_str(text);chunks+=1;}if events.send(event).is_err(){token.cancel();}};
        if use_cloud{
            metric_model = prefs.cloud_model.clone();
            if !request.automatic {
                let gate=policy::cloud_gate(prefs.usage_mode,request.sensitivity,request.reviewed,consent,request.request_cloud,&format!("{}\n{}\n{}",prefs.personality,request.message,referenced));
                if !gate.allowed { return Err(gate.reason); }
                if request.personality_preview.as_deref()!=Some(prefs.system_prompt()) { return Err("A personalidade mudou. Revise novamente antes de enviar à nuvem.".into()); }
            }
            let effort=match request.effort.as_deref(){None|Some("")=>None,Some(e@("low"|"medium"|"high"))=>Some(e),Some(_)=>return Err("Esforço inválido".into())};
            // Nunca enviar histórico, memórias, anexos ou dados de órgãos.
            let platform=prefs.cloud_platform().ok_or("Escolha uma plataforma em Configurações › Nuvem")?.to_string();
            let (api,base,secret)=state.cloud_secret(&prefs,&platform).await?;
            // Modelo automático por padrão (decisão do usuário, 21/11/2026): o escolhido nos ajustes
            // desta resposta; senão o do provedor próprio (Opções avançadas) ou o padrão da plataforma;
            // senão o primeiro modelo de conversa que a conta oferece.
            let picked = if request.automatic { None } else { request.cloud_model.as_deref().map(str::trim).filter(|m| !m.is_empty() && m.len() <= 200) };
            let preferred = picked.map(str::to_owned).unwrap_or_else(|| if platform == "custom" { prefs.cloud_model.clone() }
                else { cloud::provider(&platform).and_then(|p| p.default_model).unwrap_or("").to_owned() });
            let cloud_model = if !preferred.is_empty() { preferred }
                else {
                    let catalog = tokio::select! { _ = token.cancelled() => return Err("Resposta interrompida".into()), list = cloud::models(api,&base,&secret,false) => list? };
                    cloud::automatic_model(&platform,&catalog)?.to_owned()
                };
            metric_model = cloud_model.clone();
            // Referências: só memórias públicas, iguais ao revisado e do escopo desta conversa.
            let mut instructions=if request.automatic { include_str!("persona.txt").trim() } else { prefs.system_prompt() }.to_owned();
            instructions.push_str(OPTIONS_GUIDE);
            if !request.references.is_empty(){
                let db=state.store.lock().map_err(|_|"Dados ocupados")?;
                let project=db.conversation_project(&request.conversation).map_err(|_|"Conversa não encontrada")?;
                let ids:Vec<String>=request.references.iter().map(|r|r.id.clone()).collect();
                let facts=db.facts_by_ids(&ids).map_err(|_|"Falha ao consultar memórias")?;
                memory::check_references(&request.references,&facts,project.as_deref())?;
                if let Some(block)=memory::reference_block(&request.references){instructions.push_str(&block);}
            }
            origin = Some(if web_search { "cloud_search" } else { "cloud" });
            if web_search {
                let _ = events.send(json!({"type":"status","text":"Pesquisando fontes na web…"}));
                let outgoing = cloud::search_request(&platform, &secret, &cloud_model, effort, &instructions, &request.message)?;
                cloud::stream_search(outgoing, &token, super::research::wants_youtube(&request.message), &mut report).await?;
            } else {
                let outgoing=cloud::chat_request(api,&base,&secret,&cloud_model,effort,&instructions,&request.message)?;
                cloud::stream(outgoing,api,&token,&mut report).await?;
            }
        }else{
            // Sem modelo local, a pergunta sobre finanças recebe os números da Moneta diretamente.
            if selection.is_err(){if let Some(data)=&finance{
                metric_model="moneta".into();
                let text=match data{Ok(resumo)=>format!("Não há modelo local para conversar agora, então aqui estão os números da Moneta:\n\n{}",super::organs::finance_text(resumo)),Err(error)=>format!("Não consegui consultar a Moneta: {error}")};
                report(json!({"type":"token","text":text}));
                return Ok(());
            }}
            let model=match selection{Ok(model)=>model,Err(error)=>{if prefs.usage_mode!=UsageMode::LocalOnly{let _=events.send(json!({"type":"suggestion","text":"Não há modelo local compatível disponível. Você pode revisar o texto e solicitar cloud, mantendo dados sensíveis locais."}));}return Err(error);}};
            metric_model = model.id.clone();
            let _=events.send(json!({"type":"routing","text":if recalled>0{format!("{} · {:?} · {} memórias consideradas",model.name,kind,recalled)}else{format!("{} · {:?}",model.name,kind)}}));
            state.runtime.local_chat(&model,kind,&prefs,context,&token,&mut report).await?;
        }
        Ok(())
    }.await;
    if !partial.is_empty() {
        match state.store.lock() {
            Ok(db) => {
                if db
                    .save_turn(
                        &request.conversation,
                        &request.message,
                        &partial,
                        result.is_ok(),
                        &request.attachments,
                        &[],
                        origin,
                    )
                    .is_err()
                {
                    result = Err(
                        "A resposta foi recebida, mas não foi possível salvar esta conversa".into(),
                    );
                }
                if let Some(source) = origin {
                    let _ = events.send(json!({"type":"origin","origin":source}));
                }
                let _ = db.metric(
                    &metric_model,
                    first,
                    start.elapsed().as_millis() as u64,
                    chunks,
                    token.is_cancelled(),
                );
            }
            Err(_) => result = Err("A resposta foi recebida, mas os dados estão ocupados".into()),
        }
    }
    // Pedido explícito ("lembre que…", "esqueça…"): processar já, sem esperar a pausa.
    if result.is_ok() && memory::explicit_command(&request.message) && !reminders::is_request(&request.message) {
        if let Ok(prefs) = state.prefs() {
            if prefs.memory_auto {
                match state.remember(&prefs, &request.conversation, "explicit", &token).await {
                    Ok(facts) => { let _ = events.send(json!({"type":"memory","facts":facts})); }
                    Err(error) => { let _ = events.send(json!({"type":"memory_error","text":error})); }
                }
            }
        }
    }
    if let Ok(mut last) = state.last_activity.lock() {
        *last = Instant::now();
    }
    // Título: provisório agora (primeiras palavras), resumido pela Ayla em segundo plano.
    if result.is_ok() && !token.is_cancelled() {
        let current = state.store.lock().ok().and_then(|db| db.conversation_title(&request.conversation).ok());
        if current.as_deref() == Some(DEFAULT_TITLE) {
            let quick = quick_title(&request.message);
            if state.store.lock().ok().and_then(|db| db.retitle(&request.conversation, DEFAULT_TITLE, &quick).ok()) == Some(true) {
                use tauri::Emitter;
                let _ = app.emit("conversation-updated", json!({"id":request.conversation,"title":quick}));
                let (handle, conversation, message) = (app.clone(), request.conversation.clone(), request.message.clone());
                tauri::async_runtime::spawn(async move { smart_title(handle, conversation, message, quick).await });
            }
        }
    }
    // Resposta pronta: avisa se a pessoa saiu da janela (nunca com o conteúdo).
    if result.is_ok() && !token.is_cancelled() {
        if let Ok(prefs) = state.prefs() {
            notify::send(&app, &prefs, notify::Event::Reply);
        }
    }
    state.finish(&request.id);
    result
}


/// Apaga a pasta da cópia de um arquivo (`files/<id>/…`), nunca algo fora dela.
fn remove_copy(root: &std::path::Path, relative: &str) {
    let path = root.join(relative);
    if let Some(dir) = path.parent() {
        if dir.parent() == Some(root.join("files").as_path()) {
            let _ = fs::remove_dir_all(dir);
        }
    }
}
/// Ao abrir: descarta anexos nunca enviados (mais de um dia) e cópias sem registro.
fn clean_files(root: &std::path::Path, store: &Store) {
    let _ = store.forget_unsent_files(store::now() - 86_400);
    let Ok(known) = store.file_paths() else { return };
    let known: std::collections::HashSet<String> = known.iter().filter_map(|p| p.split('/').nth(1).map(String::from)).collect();
    if let Ok(entries) = fs::read_dir(root.join("files")) {
        for entry in entries.flatten() {
            if !known.contains(&entry.file_name().to_string_lossy().into_owned()) {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }
}

/// Resultado do preparo de um anexo: texto para o prompt e imagens para a visão.
struct Prepared {
    text: Option<String>,
    images: Vec<PathBuf>,
}
impl Core {
    /// Anexos do pedido: existentes, do mesmo espaço da conversa e ainda não enviados.
    fn attachments(&self, request: &ChatRequest) -> Result<Vec<FileInfo>, String> {
        if request.attachments.len() > 5 {
            return Err("Envie até 5 arquivos por mensagem".into());
        }
        let db = self.store.lock().map_err(|_| "Dados ocupados")?;
        let project = db.conversation_project(&request.conversation).map_err(|_| "Conversa não encontrada")?;
        request.attachments.iter().map(|id| {
            let file = db.file(id).ok().flatten().ok_or("Um arquivo anexado não foi encontrado; anexe de novo")?;
            if file.project_id != project || file.message_id.is_some() {
                return Err("Este arquivo pertence a outro espaço ou já foi enviado".to_string());
            }
            Ok(file)
        }).collect()
    }
    /// ffmpeg instalado, ou baixado agora (com progresso na conversa).
    async fn ffmpeg(&self, token: &CancellationToken, report: &mut impl FnMut(Value)) -> Result<PathBuf, String> {
        if let Some(path) = self.runtime.tool("ffmpeg", "ffmpeg") {
            return Ok(path);
        }
        report(json!({"type":"status","text":"Preparando o leitor de áudio e vídeo…"}));
        runtime::install(&self.root.join("runtimes"), "ffmpeg", token, |event| {
            if let (Some(received), Some(total)) = (event["received"].as_u64(), event["total"].as_u64()) {
                report(json!({"type":"status","text":format!("Preparando o leitor de áudio e vídeo · {} de {} MB", received >> 20, total >> 20)}));
            }
        }).await?;
        self.runtime.tool("ffmpeg", "ffmpeg").ok_or_else(|| "O leitor de áudio e vídeo não foi instalado".into())
    }
    /// Lê, transcreve ou extrai quadros de um anexo; resultados ficam em cache.
    async fn prepare_file(&self, prefs: &Preferences, file: &FileInfo, token: &CancellationToken, report: &mut impl FnMut(Value)) -> Result<Prepared, String> {
        let source = self.root.join(&file.path);
        let dir = source.parent().ok_or("Arquivo inválido")?.to_path_buf();
        if !source.is_file() {
            return Err(format!("A cópia de {} não está mais disponível", file.name));
        }
        match file.kind {
            Kind::Text | Kind::Pdf => {
                let text = match file.extracted.clone() {
                    Some(text) => text,
                    None => {
                        report(json!({"type":"status","text":format!("Lendo {}…", file.name)}));
                        let (path, kind) = (source.clone(), file.kind);
                        let text = tauri::async_runtime::spawn_blocking(move || if kind == Kind::Pdf { files::pdf_text(&path) } else { files::text_file(&path).map_err(|_| "Não foi possível ler o arquivo".to_string()) })
                            .await.map_err(|_| "Falha ao ler arquivo")??;
                        if let Ok(db) = self.store.lock() { let _ = db.set_extracted(&file.id, &text); }
                        text
                    }
                };
                let text = if text.trim().is_empty() { "(sem texto legível; pode ser um documento escaneado)".to_string() } else { text };
                Ok(Prepared { text: Some(text), images: Vec::new() })
            }
            Kind::Image => {
                let mut head = [0u8; 4];
                let read = std::io::Read::read(&mut fs::File::open(&source).map_err(|_| "Não foi possível ler a imagem")?, &mut head).unwrap_or(0);
                if head[..read].starts_with(b"\x89PNG") || head[..read].starts_with(&[0xff, 0xd8]) {
                    return Ok(Prepared { text: None, images: vec![source] });
                }
                let converted = dir.join("imagem.png");
                if !converted.is_file() {
                    let ffmpeg = self.ffmpeg(token, report).await?;
                    let mut command = std::process::Command::new(ffmpeg);
                    command.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-i"]).arg(&source).args(["-frames:v", "1", "-vf", "scale='min(1600,iw)':-2", "-y"]).arg(&converted);
                    runtime::run_tool(command, token, Duration::from_secs(60)).await?;
                }
                if !converted.is_file() {
                    return Err(format!("Não foi possível ler a imagem {}", file.name));
                }
                Ok(Prepared { text: None, images: vec![converted] })
            }
            Kind::Gif => Ok(Prepared { text: None, images: self.frames(file, &source, &dir, token, report).await? }),
            Kind::Audio => Ok(Prepared { text: Some(self.transcript(prefs, file, &source, &dir, token, report).await?.unwrap_or_else(|| "(sem fala reconhecível)".into())), images: Vec::new() }),
            Kind::Video => {
                let transcript = self.transcript(prefs, file, &source, &dir, token, report).await?;
                let images = self.frames(file, &source, &dir, token, report).await?;
                Ok(Prepared { text: Some(transcript.unwrap_or_else(|| "(vídeo sem áudio ou sem fala reconhecível)".into())), images })
            }
        }
    }
    /// Até 4 quadros representativos (vídeo ao longo da duração; GIF no início).
    async fn frames(&self, file: &FileInfo, source: &std::path::Path, dir: &std::path::Path, token: &CancellationToken, report: &mut impl FnMut(Value)) -> Result<Vec<PathBuf>, String> {
        let frame = |i: usize| dir.join(format!("quadro-{i}.jpg"));
        let existing: Vec<PathBuf> = (0..files::MAX_IMAGES).map(frame).filter(|p| p.is_file()).collect();
        if !existing.is_empty() {
            return Ok(existing);
        }
        let ffmpeg = self.ffmpeg(token, report).await?;
        report(json!({"type":"status","text":format!("Vendo {}…", file.name)}));
        let scale = "scale='min(1024,iw)':-2";
        if file.kind == Kind::Gif {
            let mut command = std::process::Command::new(&ffmpeg);
            command.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-i"]).arg(source).args(["-vf", &format!("fps=2,{scale}"), "-frames:v", "4", "-q:v", "3", "-start_number", "0", "-y"]).arg(dir.join("quadro-%d.jpg"));
            runtime::run_tool(command, token, Duration::from_secs(60)).await?;
        } else {
            let mut probe = std::process::Command::new(&ffmpeg);
            probe.args(["-nostdin", "-hide_banner", "-i"]).arg(source);
            let (_, info) = runtime::run_tool(probe, token, Duration::from_secs(30)).await?;
            let times = files::frame_times(files::parse_duration(&info).unwrap_or(0.0), files::MAX_IMAGES);
            for (i, time) in times.iter().enumerate() {
                let mut command = std::process::Command::new(&ffmpeg);
                command.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-ss", &format!("{time:.2}"), "-i"]).arg(source).args(["-frames:v", "1", "-vf", scale, "-q:v", "3", "-y"]).arg(frame(i));
                runtime::run_tool(command, token, Duration::from_secs(60)).await?;
            }
        }
        Ok((0..files::MAX_IMAGES).map(frame).filter(|p| p.is_file()).collect())
    }
    /// Transcrição do áudio (até 10 minutos) com o Whisper; `None` se não houver fala ou áudio.
    async fn transcript(&self, prefs: &Preferences, file: &FileInfo, source: &std::path::Path, dir: &std::path::Path, token: &CancellationToken, report: &mut impl FnMut(Value)) -> Result<Option<String>, String> {
        if let Some(text) = &file.extracted {
            return Ok((!text.trim().is_empty()).then(|| text.clone()));
        }
        let model = self.transcriber(prefs, false).map_err(|_| "Para entender áudios, ative o microfone em Configurações (a transcrição roda neste computador)".to_string())?;
        let ffmpeg = self.ffmpeg(token, report).await?;
        report(json!({"type":"status","text":format!("Ouvindo {}…", file.name)}));
        let wav = dir.join("audio.wav");
        let mut command = std::process::Command::new(ffmpeg);
        command.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-i"]).arg(source).args(["-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le", "-t", "600", "-f", "wav", "-y"]).arg(&wav);
        let (converted, _) = runtime::run_tool(command, token, Duration::from_secs(300)).await?;
        if !converted || fs::metadata(&wav).map(|m| m.len() < 2048).unwrap_or(true) {
            let _ = fs::remove_file(&wav);
            if file.kind == Kind::Video {
                if let Ok(db) = self.store.lock() { let _ = db.set_extracted(&file.id, ""); }
                return Ok(None);
            }
            return Err(format!("Não foi possível ler o áudio de {}", file.name));
        }
        let base = dir.join("transcricao");
        let heard = self.runtime.transcribe(&model, &wav, &base, token).await;
        let _ = fs::remove_file(&wav);
        heard?;
        let text = fs::read_to_string(base.with_extension("txt")).unwrap_or_default();
        let _ = fs::remove_file(base.with_extension("txt"));
        if let Ok(db) = self.store.lock() { let _ = db.set_extracted(&file.id, text.trim()); }
        Ok((!text.trim().is_empty()).then(|| text.trim().to_string()))
    }
    /// Copia um arquivo para `files/<id>/` e registra; `kept` = adicionado direto em Arquivos.
    fn store_file(&self, name: &str, write: impl FnOnce(&std::path::Path) -> Result<u64, String>, project: Option<String>, kept: bool) -> Result<FileInfo, String> {
        if let Some(id) = &project {
            self.store.lock().map_err(|_| "Dados ocupados")?.project(id).ok().flatten().ok_or("Projeto não encontrado")?;
        }
        let id = store::id();
        let dir = self.root.join("files").join(&id);
        fs::create_dir_all(&dir).map_err(|_| "Não foi possível guardar o arquivo")?;
        let name = files::safe_name(name);
        let dest = dir.join(&name);
        let result = (|| {
            let bytes = write(&dest)?;
            let mut head = [0u8; 16];
            let read = std::io::Read::read(&mut fs::File::open(&dest).map_err(|_| "Não foi possível ler o arquivo")?, &mut head).unwrap_or(0);
            let kind = files::kind_of(&name, &head[..read]).ok_or_else(|| format!("Formato não suportado: {name}. Use imagens, GIFs, PDFs, textos, áudios ou vídeos."))?;
            let sha = models::hash(&dest).map_err(|_| "Não foi possível ler o arquivo")?;
            let info = FileInfo { id: id.clone(), name: name.clone(), kind, bytes: bytes as i64, created_at: store::now(), conversation_id: None, project_id: project, message_id: None, path: format!("files/{id}/{name}"), extracted: None };
            self.store.lock().map_err(|_| "Dados ocupados")?.insert_file(&info, &sha, kept).map_err(|_| "Não foi possível registrar o arquivo")?;
            Ok(info)
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&dir);
        }
        result
    }
}

/// Escolher arquivos para anexar (imagens, GIFs, PDFs, textos, áudios e vídeos).
#[tauri::command]
pub async fn pick_files(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    let all: Vec<&'static str> = [files::IMAGE, files::TEXT, files::AUDIO, files::VIDEO, &["pdf", "gif"]].concat();
    let handle = app.clone();
    crate::desktop::panel_dialog(&handle, move || {
        app.dialog().file().add_filter("Arquivos suportados", &all).blocking_pick_files()
            .unwrap_or_default().into_iter().filter_map(|p| p.into_path().ok()).map(|p| p.to_string_lossy().into_owned()).collect()
    })
    .await
}
/// Guarda uma cópia de um arquivo do computador (o original não é alterado).
/// `keep`: adicionado direto em Arquivos do projeto; senão, anexo da caixa de mensagem.
#[tauri::command]
pub async fn add_file(app: tauri::AppHandle, path: String, project: Option<String>, keep: Option<bool>) -> Result<FileInfo, String> {
    let source = PathBuf::from(&path);
    let meta = fs::metadata(&source).map_err(|_| "Arquivo não encontrado")?;
    if !meta.is_file() {
        return Err("Escolha um arquivo, não uma pasta".into());
    }
    if meta.len() > files::MAX_BYTES {
        return Err("O arquivo passa de 1 GB".into());
    }
    let name = source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "arquivo".into());
    // Cópia de arquivos grandes fora da thread da interface.
    tauri::async_runtime::spawn_blocking(move || {
        use tauri::Manager;
        let core = app.state::<Core>();
        core.store_file(&name, |dest| fs::copy(&source, dest).map_err(|_| "Não foi possível copiar o arquivo".to_string()), project, keep.unwrap_or(false))
    })
    .await
    .map_err(|_| "Falha ao guardar o arquivo")?
}
/// Arquivo vindo da área de transferência (ex.: imagem colada).
#[tauri::command]
pub fn add_file_data(state: tauri::State<'_, Core>, name: String, data: String, project: Option<String>) -> Result<FileInfo, String> {
    if data.len() > 90 * 1024 * 1024 {
        return Err("Arquivo grande demais para colar; use o botão de anexar".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|_| "Arquivo inválido")?;
    state.store_file(&name, |dest| fs::write(dest, &bytes).map(|_| bytes.len() as u64).map_err(|_| "Não foi possível guardar o arquivo".to_string()), project, false)
}
#[tauri::command]
pub fn remove_file(state: tauri::State<'_, Core>, id: String) -> Result<(), String> {
    let path = state.store.lock().map_err(|_| "Dados ocupados")?.delete_file(&id).map_err(|_| "Arquivo não encontrado")?;
    if let Some(path) = path {
        remove_copy(&state.root, &path);
    }
    Ok(())
}
/// Arquivos guardados de um projeto (ou gerais), para a aba Arquivos.
#[tauri::command]
pub fn project_files(state: tauri::State<'_, Core>, project: Option<String>) -> Result<Vec<FileInfo>, String> {
    state.store.lock().map_err(|_| "Dados ocupados")?.project_files(project.as_deref()).map_err(|_| "Falha ao ler arquivos".into())
}

impl Core {
    /// Segredo guardado nesta sessão ou no cofre do sistema.
    fn vault_get(&self, account: &str) -> Option<String> {
        if let Some(value) = self.keys.lock().ok()?.get(account) {
            return Some(value.clone());
        }
        keyring::Entry::new("dev.feit.ayla", account).and_then(|e| e.get_password()).ok()
    }
    /// Guarda no cofre (`remember`) ou só nesta sessão. Sem cofre, avisa e mantém na sessão.
    fn vault_set(&self, account: &str, secret: &str, remember: bool) -> Result<(), String> {
        self.keys.lock().map_err(|_| "Credencial ocupada")?.insert(account.to_string(), secret.to_string());
        if remember {
            keyring::Entry::new("dev.feit.ayla", account).and_then(|e| e.set_password(secret))
                .map_err(|_| "O cofre do sistema não está disponível; a conexão vale só até fechar a Ayla.")?;
        }
        Ok(())
    }
    fn vault_delete(&self, account: &str) -> Result<(), String> {
        self.keys.lock().map_err(|_| "Credencial ocupada")?.remove(account);
        match keyring::Entry::new("dev.feit.ayla", account).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err("Desconectado nesta sessão, mas não foi possível confirmar a remoção no cofre do sistema".into()),
        }
    }
    fn chatgpt_session(&self) -> Option<cloud::ChatGptSession> {
        serde_json::from_str(&self.vault_get(&cloud::vault_account("chatgpt"))?).ok()
    }
    /// Identificador estável e aleatório deste computador para o "Entrar com ChatGPT".
    fn host_id(&self) -> Result<String, String> {
        let path = self.root.join("host-id");
        if let Ok(id) = fs::read_to_string(&path) {
            if id.starts_with("urn:uuid:") {
                return Ok(id.trim().to_string());
            }
        }
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|_| "Sem fonte de números aleatórios do sistema")?;
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let id = format!("urn:uuid:{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32]);
        fs::write(&path, &id).map_err(|_| "Não foi possível guardar a identificação deste computador")?;
        Ok(id)
    }
    /// API, endereço e credencial para falar com a plataforma (renova o acesso do ChatGPT).
    async fn cloud_secret(&self, prefs: &Preferences, platform: &str) -> Result<(Api, String, Secret), String> {
        if platform == "custom" {
            return Ok((Api::Compatible, prefs.cloud_endpoint.clone(), Secret::Key(self.key()?)));
        }
        let provider = cloud::provider(platform).ok_or("Plataforma de nuvem desconhecida")?;
        if provider.login != Login::ChatGpt {
            let key = self.vault_get(&cloud::vault_account(provider.id)).ok_or_else(|| format!("Conecte {} em Configurações › Nuvem", provider.name))?;
            return Ok((provider.api, provider.base.to_string(), Secret::Key(key)));
        }
        let _refresh = self.cloud_gate.lock().await;
        let session = self.chatgpt_session().ok_or("Entre com o ChatGPT em Configurações › Nuvem")?;
        if !session.needs_refresh(store::now()) {
            return Ok((Api::ChatGpt, provider.base.to_string(), Secret::ChatGpt(session.access_token)));
        }
        match cloud::chatgpt_refresh(&session, store::now()).await {
            Ok(renewed) => {
                let saved = serde_json::to_string(&renewed).map_err(|_| "Credencial inválida")?;
                self.vault_set(&cloud::vault_account("chatgpt"), &saved, true).or_else(|_| self.vault_set(&cloud::vault_account("chatgpt"), &saved, false))?;
                Ok((Api::ChatGpt, provider.base.to_string(), Secret::ChatGpt(renewed.access_token)))
            }
            Err((terminal, message)) => {
                if terminal {
                    let _ = self.vault_delete(&cloud::vault_account("chatgpt"));
                }
                Err(message)
            }
        }
    }
}

/// Plataformas de nuvem e se cada uma está conectada (consulta o cofre do sistema).
#[tauri::command]
pub fn cloud_providers(state: tauri::State<'_, Core>) -> Result<Value, String> {
    Ok(json!(cloud::providers().iter().map(|p| {
        let (connected, account) = if p.login == Login::ChatGpt {
            let session = state.chatgpt_session();
            (session.is_some(), session.and_then(|s| s.email))
        } else {
            (state.vault_get(&cloud::vault_account(p.id)).is_some(), None)
        };
        json!({"id":p.id,"name":p.name,"summary":p.summary,"login":p.login,"keys_url":p.keys_url,"default_model":p.default_model,"images":p.images,"connected":connected,"account":account})
    }).collect::<Vec<_>>()))
}
/// Conecta uma plataforma por chave de API (guardada no cofre ou só nesta sessão).
#[tauri::command]
pub fn cloud_connect(state: tauri::State<'_, Core>, provider: String, key: String, remember: bool) -> Result<(), String> {
    let platform = cloud::provider(&provider).ok_or("Plataforma de nuvem desconhecida")?;
    let key = key.trim();
    if platform.login == Login::ChatGpt || key.len() < 8 || key.len() > 4096 || key.chars().any(char::is_whitespace) {
        return Err("Cole a chave de API completa, sem espaços".into());
    }
    state.vault_set(&cloud::vault_account(platform.id), key, remember)
}
/// Login pelo navegador: OpenRouter (chave da conta) ou "Entrar com ChatGPT" (plano).
#[tauri::command]
pub async fn cloud_login(state: tauri::State<'_, Core>, provider: String, request_id: String) -> Result<Value, String> {
    let platform = cloud::provider(&provider).ok_or("Plataforma de nuvem desconhecida")?;
    let token = state.job(&request_id)?;
    let result = async {
        match platform.login {
            Login::OpenRouter => {
                let key = cloud::openrouter_login(&token).await?;
                let remembered = state.vault_set(&cloud::vault_account(platform.id), &key, true);
                Ok(json!({"account":null,"warning":remembered.err()}))
            }
            Login::ChatGpt => {
                let host = state.host_id()?;
                let previous = state.chatgpt_session();
                let session = cloud::chatgpt_login(&host, previous.as_ref(), &token, store::now()).await?;
                let saved = serde_json::to_string(&session).map_err(|_| "Credencial inválida")?;
                let remembered = state.vault_set(&cloud::vault_account("chatgpt"), &saved, true);
                Ok(json!({"account":session.email,"warning":remembered.err()}))
            }
            Login::Key => Err("Esta plataforma usa chave de API".into()),
        }
    }
    .await;
    state.finish(&request_id);
    result
}
#[tauri::command]
pub async fn cloud_disconnect(state: tauri::State<'_, Core>, provider: String) -> Result<(), String> {
    let platform = cloud::provider(&provider).ok_or("Plataforma de nuvem desconhecida")?;
    if platform.login == Login::ChatGpt {
        if let Some(session) = state.chatgpt_session() {
            cloud::chatgpt_revoke(&session).await;
        }
    }
    state.vault_delete(&cloud::vault_account(platform.id))
}
/// Modelos disponíveis na conta conectada.
#[tauri::command]
pub async fn cloud_models(state: tauri::State<'_, Core>, provider: String, images: Option<bool>) -> Result<Value, String> {
    let prefs = state.prefs()?;
    let (api, base, secret) = state.cloud_secret(&prefs, &provider).await?;
    let list = cloud::models(api, &base, &secret, images.unwrap_or(false)).await?;
    Ok(json!(list.into_iter().map(|(id, name)| json!({"id":id,"name":name})).collect::<Vec<_>>()))
}
#[derive(Deserialize)]
pub struct ImageRequest {
    pub id: String,
    pub conversation: String,
    pub prompt: String,
    pub sensitivity: Sensitivity,
    pub reviewed: bool,
}
/// Cria uma imagem pela nuvem só quando a pessoa pede, com o pedido revisado e público.
/// A imagem fica guardada na Ayla, ligada à resposta da conversa (e ao projeto).
#[tauri::command]
pub async fn generate_image(state: tauri::State<'_, Core>, request: ImageRequest) -> Result<Value, String> {
    let prompt = request.prompt.trim().to_string();
    if prompt.is_empty() || prompt.chars().count() > 4000 {
        return Err("Descreva a imagem em até 4000 caracteres".into());
    }
    let token = state.job(&request.id)?;
    let result = async {
        let prefs = state.prefs()?;
        if prefs.usage_mode == UsageMode::LocalOnly {
            return Err("Ligue a nuvem em Configurações › Nuvem para criar imagens".to_string());
        }
        let (consent, project) = {
            let db = state.store.lock().map_err(|_| "Dados ocupados")?;
            (db.consent(&request.conversation).map_err(|_| "Conversa não encontrada")?, db.conversation_project(&request.conversation).map_err(|_| "Conversa não encontrada")?)
        };
        let gate = policy::cloud_gate(prefs.usage_mode, request.sensitivity, request.reviewed, consent, true, &prompt);
        if !gate.allowed {
            return Err(gate.reason);
        }
        let provider = cloud::provider(&prefs.image_provider).filter(|p| p.images.is_some()).ok_or("Escolha onde criar imagens em Configurações › Nuvem")?;
        if prefs.image_model.is_empty() {
            return Err("Escolha o modelo de imagem em Configurações › Nuvem".to_string());
        }
        let key = state.vault_get(&cloud::vault_account(provider.id)).ok_or_else(|| format!("Conecte {} em Configurações › Nuvem", provider.name))?;
        let (bytes, extension) = tokio::select! {
            _ = token.cancelled() => return Err("Criação da imagem interrompida".to_string()),
            made = cloud::generate_image(provider, &key, &prefs.image_model, &prompt) => made?,
        };
        let file = state.store_file(&format!("imagem-{}.{extension}", store::now()), |dest| fs::write(dest, &bytes).map(|_| bytes.len() as u64).map_err(|_| "Não foi possível guardar a imagem".to_string()), project, false)?;
        let reply = format!("Imagem criada com {} ({}).", provider.name, prefs.image_model);
        let db = state.store.lock().map_err(|_| "Dados ocupados")?;
        db.save_turn(&request.conversation, &prompt, &reply, true, &[], &[file.id.clone()], Some("cloud")).map_err(|_| "A imagem foi criada, mas não foi possível salvar a conversa")?;
        let saved = db.file(&file.id).ok().flatten().unwrap_or(file);
        Ok(json!({"content": reply, "attachments": [saved]}))
    }
    .await;
    if let Ok(mut last) = state.last_activity.lock() {
        *last = Instant::now();
    }
    state.finish(&request.id);
    result
}
/// Imagem guardada para exibir na conversa (data URL); só imagens de até 25 MB.
#[tauri::command]
pub fn file_data(state: tauri::State<'_, Core>, id: String) -> Result<Value, String> {
    let file = state.store.lock().map_err(|_| "Dados ocupados")?.file(&id).ok().flatten().ok_or("Arquivo não encontrado")?;
    if !matches!(file.kind, Kind::Image | Kind::Gif) || file.bytes > 25 * 1024 * 1024 {
        return Err("Prévia indisponível para este arquivo".into());
    }
    let bytes = fs::read(state.root.join(&file.path)).map_err(|_| "Arquivo não encontrado")?;
    let mime = match cloud::image_extension(&bytes) { Some("png") => "image/png", Some("jpg") => "image/jpeg", Some("webp") => "image/webp", _ if bytes.starts_with(b"GIF8") => "image/gif", _ => "image/bmp" };
    Ok(json!({"url": format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))}))
}
/// Salva uma cópia de um arquivo guardado onde a pessoa escolher.
#[tauri::command]
pub async fn export_file(app: tauri::AppHandle, state: tauri::State<'_, Core>, id: String) -> Result<bool, String> {
    let file = state.store.lock().map_err(|_| "Dados ocupados")?.file(&id).ok().flatten().ok_or("Arquivo não encontrado")?;
    let source = state.root.join(&file.path);
    let name = file.name.clone();
    let handle = app.clone();
    let target = crate::desktop::panel_dialog(&handle, move || app.dialog().file().set_file_name(&name).blocking_save_file()).await?;
    let Some(target) = target else { return Ok(false) };
    let target = target.into_path().map_err(|_| "Local inválido")?;
    tauri::async_runtime::spawn_blocking(move || fs::copy(&source, &target)).await.map_err(|_| "Falha ao salvar")?.map_err(|_| "Não foi possível salvar o arquivo")?;
    Ok(true)
}
/// Link clicado numa resposta: só http/https, no navegador do sistema.
#[tauri::command]
pub fn open_web(url: String) -> Result<(), String> {
    let parsed = reqwest::Url::parse(&url).map_err(|_| "Link inválido")?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("Só links da web podem ser abertos".into());
    }
    cloud::open_url(parsed.as_str())
}
/// Abre só páginas conhecidas (chaves das plataformas e uso do ChatGPT).
#[tauri::command]
pub fn open_link(url: String) -> Result<(), String> {
    if !cloud::allowed_link(&url) {
        return Err("Link não permitido".into());
    }
    cloud::open_url(&url)
}

#[tauri::command]
pub async fn backup_data(state: tauri::State<'_, Core>) -> Result<(), String> {
    let root = state.root.join("backups");
    fs::create_dir_all(&root).map_err(|_| "Não foi possível criar backup")?;
    let name = format!("ayla-{}", store::id());
    state
        .store
        .lock()
        .map_err(|_| "Dados ocupados")?
        .backup(&root.join(format!("{name}.db")))
        .map_err(|_| "Não foi possível criar backup")?;
    // Arquivos guardados: link físico (sem duplicar espaço; sobrevive se o original for apagado) ou cópia.
    let (source, target) = (state.root.join("files"), root.join(format!("{name}-arquivos")));
    if source.is_dir() {
        tauri::async_runtime::spawn_blocking(move || copy_tree(&source, &target)).await.map_err(|_| "Falha no backup dos arquivos")?.map_err(|_| "Não foi possível copiar os arquivos para o backup".to_string())?;
    }
    Ok(())
}
fn copy_tree(source: &std::path::Path, target: &std::path::Path) -> std::io::Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let destination = target.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &destination)?;
        } else if kind.is_file() && fs::hard_link(entry.path(), &destination).is_err() {
            fs::copy(entry.path(), &destination)?;
        }
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct CapabilityRequest {
    pub id: String,
    pub text: String,
    pub sensitivity: Sensitivity,
    pub reviewed: bool,
    pub request_cloud: bool,
    pub cloud_model: Option<String>,
}

async fn cloud_embedding(
    state: &Core,
    prefs: &Preferences,
    request: &CapabilityRequest,
    token: &CancellationToken,
) -> Result<(String, Vec<f32>), String> {
    let gate = policy::cloud_gate(
        prefs.usage_mode,
        request.sensitivity,
        request.reviewed,
        true,
        request.request_cloud,
        &request.text,
    );
    if !gate.allowed {
        return Err(gate.reason);
    }
    let model = request
        .cloud_model
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&prefs.cloud_model);
    if model.is_empty() || prefs.cloud_endpoint.is_empty() {
        return Err("Configure o provedor e informe o modelo de memória cloud".into());
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|_| "Falha no cliente cloud")?;
    let response = tokio::select! {_=token.cancelled()=>return Err("Tarefa interrompida".into()),r=client.post(format!("{}/embeddings",prefs.cloud_endpoint.trim_end_matches('/'))).bearer_auth(state.key()?).json(&json!({"model":model,"input":request.text})).send()=>r.map_err(|_|"Não foi possível consultar modelo cloud")?};
    let data: Value = response
        .error_for_status()
        .map_err(|_| "Provedor não aceitou o modelo de memória")?
        .json()
        .await
        .map_err(|_| "Resposta inválida")?;
    let vector: Vec<f32> = serde_json::from_value(data["data"][0]["embedding"].clone())
        .map_err(|_| "Vetor inválido")?;
    if vector.is_empty() || vector.iter().any(|n| !n.is_finite()) {
        return Err("Vetor inválido".into());
    }
    Ok((format!("cloud:{}:{model}", prefs.cloud_endpoint), vector))
}

#[tauri::command]
pub async fn semantic_memory(
    state: tauri::State<'_, Core>,
    request: CapabilityRequest,
    search: bool,
) -> Result<Value, String> {
    if request.text.trim().is_empty() || request.text.len() > 100000 {
        return Err("Texto vazio ou muito longo".into());
    }
    let _gate = state
        .runtime
        .gate
        .try_lock()
        .map_err(|_| "Já existe uma tarefa em andamento")?;
    let token = state.job(&request.id)?;
    let result = async {
        let prefs = state.prefs()?;
        let use_cloud = prefs.usage_mode == UsageMode::CloudOnly || request.request_cloud;
        // Prefixos de busca só no modelo local; a nuvem recebe exatamente o texto revisado.
        let text = if search { memory::query_text(&request.text) } else { memory::document_text(&request.text) };
        let (model_id, vector) = if use_cloud {
            cloud_embedding(&state, &prefs, &request, &token).await?
        } else {
            let model = state.memory_model(&prefs).ok_or("Ative a memória em Configurações para usar esta ferramenta")?;
            state.backfill(&prefs, &model, &token).await?;
            let vector = state.runtime.embed(&model, &prefs, &text, &token).await?;
            (model.id, vector)
        };
        if !search {
            let db = state.store.lock().map_err(|_| "Dados ocupados")?;
            let fact = db.insert_fact(request.text.trim(), "nota", Privacy::Uncertain, "manual", None, None).map_err(|_| "Não foi possível salvar memória")?;
            db.set_vector(&fact.id, &model_id, &vector).map_err(|_| "Não foi possível salvar memória")?;
            return Ok(json!({"saved":true}));
        }
        let results: Vec<Value> = state.search(&model_id, &vector, &request.text, 0.0, 5, Scope::visible(None))?
            .into_iter()
            .map(|(fact, score)| json!({"content":fact.content,"score":score}))
            .collect();
        Ok(json!({"results":results}))
    }
    .await;
    state.finish(&request.id);
    result
}

#[tauri::command]
pub fn memory_list(state: tauri::State<'_, Core>, project: Option<String>) -> Result<Value, String> {
    let prefs = state.prefs()?;
    let facts = state.store.lock().map_err(|_| "Dados ocupados")?.facts(project.as_deref()).map_err(|_| "Falha ao consultar memórias")?;
    Ok(json!({"facts":facts,"enabled":prefs.memory_auto,"ready":state.memory_model(&prefs).is_some()}))
}
#[tauri::command]
pub fn conversation_memories(state: tauri::State<'_, Core>, id: String) -> Result<Vec<Fact>, String> {
    state.store.lock().map_err(|_| "Dados ocupados")?.conversation_facts(&id).map_err(|_| "Falha ao consultar memórias".into())
}
/// Cria (sem `id`) ou edita uma memória pelo painel. O vetor é recalculado depois, em segundo plano.
#[tauri::command]
pub fn memory_save(state: tauri::State<'_, Core>, id: Option<String>, content: String, privacy: Privacy, project: Option<String>) -> Result<(), String> {
    let content = content.trim();
    if content.chars().count() < 3 || content.chars().count() > 1000 {
        return Err("A memória deve ter entre 3 e 1000 caracteres".into());
    }
    let db = state.store.lock().map_err(|_| "Dados ocupados")?;
    match id {
        Some(id) => db.update_fact(&id, Some(content), Some(privacy)).map_err(|_| "Memória não encontrada".to_string()),
        None => {
            if let Some(id) = &project {
                db.project(id).ok().flatten().ok_or("Projeto não encontrado")?;
            }
            db.insert_fact(content, "fato", privacy, "manual", None, project.as_deref()).map(|_| ()).map_err(|_| "Não foi possível salvar memória".to_string())
        }
    }
}
#[tauri::command]
pub fn memory_delete(state: tauri::State<'_, Core>, id: String) -> Result<(), String> {
    state.store.lock().map_err(|_| "Dados ocupados")?.delete_fact(&id).map_err(|_| "Não foi possível apagar".to_string())?
        .then_some(())
        .ok_or_else(|| "Memória não encontrada".into())
}

/// Chamado periodicamente: extrai memórias de conversas paradas e completa vetores.
/// Lembretes vencidos: avisa e, se repetem, avança para a próxima vez no futuro; senão, saem.
/// Roda com a Ayla em segundo plano; os perdidos com o computador desligado avisam ao abrir.
pub async fn reminder_tick(app: &tauri::AppHandle) {
    use tauri::{Emitter, Manager};
    let Some(state) = app.try_state::<Core>() else { return };
    let Ok(prefs) = state.prefs() else { return };
    let now = reminders::to_unix(reminders::now_local());
    let Ok(due) = state.store.lock().map(|db| db.reminders(Some(now)).unwrap_or_default()) else { return };
    if due.is_empty() {
        return;
    }
    for reminder in due {
        notify::reminder(app, &prefs, &reminder.text, now - reminder.due > 180);
        let repeat = reminders::Repeat::parse(&reminder.repeat);
        let Ok(db) = state.store.lock() else { return };
        if repeat == reminders::Repeat::None {
            let _ = db.delete_reminder(&reminder.id);
        } else {
            let mut next = reminders::from_unix(reminder.due);
            while reminders::to_unix(next) <= now {
                next = reminders::next(next, repeat);
            }
            let _ = db.reschedule_reminder(&reminder.id, reminders::to_unix(next));
        }
    }
    let _ = app.emit("reminders-updated", ());
}

#[derive(serde::Serialize)]
pub struct ReminderView {
    pub id: String,
    pub text: String,
    pub due: i64,
    pub repeat: String,
    /// "amanhã às 09:00 (todo dia)".
    pub label: String,
}

#[tauri::command]
pub fn reminders_list(state: tauri::State<'_, Core>) -> Result<Vec<ReminderView>, String> {
    let now = reminders::now_local();
    let list = state.store.lock().map_err(|_| "Dados ocupados")?.reminders(None).map_err(|_| "Falha ao consultar lembretes")?;
    Ok(list.into_iter().map(|r| {
        let when = reminders::from_unix(r.due);
        let label = format!("{}{}", reminders::describe(when, now), reminders::repeat_label(reminders::Repeat::parse(&r.repeat), when));
        ReminderView { id: r.id, text: r.text, due: r.due, repeat: r.repeat, label }
    }).collect())
}

#[tauri::command]
pub fn reminder_delete(app: tauri::AppHandle, state: tauri::State<'_, Core>, id: String) -> Result<(), String> {
    use tauri::Emitter;
    state.store.lock().map_err(|_| "Dados ocupados")?.delete_reminder(&id).map_err(|_| "Falha ao cancelar o lembrete")?;
    let _ = app.emit("reminders-updated", ());
    Ok(())
}

/// Adia a próxima vez (a partir de agora, se já tiver passado).
#[tauri::command]
pub fn reminder_snooze(app: tauri::AppHandle, state: tauri::State<'_, Core>, id: String, minutes: i64) -> Result<(), String> {
    use tauri::Emitter;
    if !(1..=7 * 24 * 60).contains(&minutes) {
        return Err("Adiamento inválido".into());
    }
    let db = state.store.lock().map_err(|_| "Dados ocupados")?;
    let current = db.reminders(None).map_err(|_| "Falha ao consultar lembretes")?.into_iter().find(|r| r.id == id).ok_or("Lembrete não encontrado")?;
    let now = reminders::to_unix(reminders::now_local());
    db.reschedule_reminder(&id, current.due.max(now) + minutes * 60).map_err(|_| "Falha ao adiar o lembrete")?;
    let _ = app.emit("reminders-updated", ());
    Ok(())
}

pub async fn memory_tick(app: &tauri::AppHandle) {
    use tauri::{Emitter, Manager};
    let Some(state) = app.try_state::<Core>() else { return };
    let Ok(prefs) = state.prefs() else { return };
    let idle = state.last_activity.lock().map(|t| t.elapsed() >= Duration::from_secs(memory::IDLE_SECONDS)).unwrap_or(false);
    if !prefs.memory_auto || !idle {
        return;
    }
    let Ok(_gate) = state.runtime.gate.try_lock() else { return };
    let pending = state.store.lock().ok().and_then(|db| db.pending_extraction().ok().flatten());
    let Some((conversation, _, until)) = pending else { return };
    if state.memory_model(&prefs).is_none() { return; }
    let token = CancellationToken::new();
    if let Ok(mut slot) = state.background.lock() {
        *slot = Some(token.clone());
    }
    let result = state.remember(&prefs, &conversation, "auto", &token).await;
    if let Ok(mut slot) = state.background.lock() {
        *slot = None;
    }
    match result {
        Ok(facts) => { let _ = app.emit("memory-updated", json!({"conversation":conversation,"facts":facts})); }
        // Sem modelo de conversa local, a extração aguarda; outros erros não repetem em laço.
        Err(_) if token.is_cancelled() => {}
        Err(_) => state.advance_failed_extraction(&prefs, &conversation, until),
    }
}

/// Modelo de transcrição só para prévia ao vivo; nunca usado no texto final.
pub const PREVIEW_TRANSCRIBER: &str = "whisper-tiny";
impl Core {
    fn advance_failed_extraction(&self, prefs: &Preferences, conversation: &str, until: i64) {
        // Consultar os modelos antes de travar o banco para escrever o cursor.
        let Ok(models) = self.models() else { return };
        if models::choose(&models, Capability::Text, self.budget(prefs), prefs.context_tokens, None).is_ok() {
            if let Ok(db) = self.store.lock() {
                let _ = db.set_memory_cursor(conversation, until);
            }
        }
    }
    /// Transcritor final (ou de prévia). Sem o preferido, usa o outro disponível.
    fn transcriber(&self, prefs: &Preferences, preview: bool) -> Result<Model, String> {
        let all = self.models()?;
        let (preferred, other): (Vec<Model>, Vec<Model>) = all.into_iter().partition(|m| (m.catalog.as_deref() == Some(PREVIEW_TRANSCRIBER)) == preview);
        let budget = self.budget(prefs);
        models::choose(&preferred, Capability::Transcription, budget, prefs.context_tokens, None)
            .or_else(|_| models::choose(&other, Capability::Transcription, budget, prefs.context_tokens, None))
            .map_err(|_| "Ative a voz em Configurações para usar o microfone".into())
    }
}

/// Prévia do que está sendo falado (Whisper tiny), chamada durante a gravação.
/// Se a voz estiver ocupada, devolve vazio em vez de esperar: a próxima prévia virá.
#[tauri::command]
pub async fn transcribe_preview(state: tauri::State<'_, Core>, request_id: String, audio: String) -> Result<Value, String> {
    if audio.len() > 16 * 1024 * 1024 {
        return Err("Áudio muito grande para prévia".into());
    }
    let Ok(_voice) = state.voice_gate.try_lock() else { return Ok(json!({"text":null})) };
    let token = state.job(&request_id)?;
    let work = state.root.join("work").join(store::id());
    let result = async {
        let prefs = state.prefs()?;
        let model = state.transcriber(&prefs, true)?;
        fs::create_dir_all(&work).map_err(|_| "Não foi possível preparar a prévia")?;
        let (raw, input) = (work.join("recording.wav"), work.join("input.wav"));
        fs::write(&raw, base64::engine::general_purpose::STANDARD.decode(audio).map_err(|_| "Áudio inválido")?).map_err(|_| "Não foi possível preparar a prévia")?;
        let normalized = input.clone();
        tauri::async_runtime::spawn_blocking(move || normalize_wav(&raw, &normalized)).await.map_err(|_| "Falha ao preparar áudio")??;
        let base = work.join("preview");
        state.runtime.transcribe(&model, &input, &base, &token).await?;
        let text = fs::read_to_string(base.with_extension("txt")).unwrap_or_default();
        Ok(json!({"text":text.trim()}))
    }
    .await;
    let _ = fs::remove_dir_all(work);
    state.finish(&request_id);
    result
}

pub(super) fn normalize_wav(
    input: &std::path::Path,
    output: &std::path::Path,
) -> Result<(), String> {
    if fs::metadata(input)
        .map_err(|_| "Áudio não encontrado")?
        .len()
        > 64 * 1024 * 1024
    {
        return Err("Use um áudio WAV de até 64 MB".into());
    }
    let mut reader =
        hound::WavReader::open(input).map_err(|_| "Use um arquivo de áudio WAV válido")?;
    let spec = reader.spec();
    if spec.channels == 0
        || spec.sample_rate == 0
        || reader.duration() > spec.sample_rate.saturating_mul(600)
    {
        return Err("Áudio inválido ou maior que dez minutos".into());
    }
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap_or(0.0)).collect(),
        hound::SampleFormat::Int => {
            let max = (2f32).powi(i32::from(spec.bits_per_sample) - 1);
            reader
                .samples::<i32>()
                .map(|s| s.unwrap_or(0) as f32 / max)
                .collect()
        }
    };
    let mono: Vec<f32> = samples
        .chunks(usize::from(spec.channels))
        .map(|frame| frame.iter().sum::<f32>() / f32::from(spec.channels))
        .collect();
    let mut writer = hound::WavWriter::create(
        output,
        hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .map_err(|_| "Não foi possível preparar áudio")?;
    let length = mono.len() as u64 * 16000 / u64::from(spec.sample_rate);
    for i in 0..length {
        let position = i as f64 * f64::from(spec.sample_rate) / 16000.0;
        let index = position as usize;
        let fraction = (position - index as f64) as f32;
        let value = mono.get(index).copied().unwrap_or(0.0) * (1.0 - fraction)
            + mono.get(index + 1).copied().unwrap_or(0.0) * fraction;
        writer
            .write_sample((value.clamp(-1.0, 1.0) * 32767.0) as i16)
            .map_err(|_| "Falha no áudio")?;
    }
    writer
        .finalize()
        .map_err(|_| "Falha ao concluir áudio".to_string())
}

/// Transcrição local de uma gravação do microfone ou de um arquivo WAV escolhido.
/// Áudio nunca vai para a nuvem: o conteúdo falado ainda não foi verificado.
#[tauri::command]
pub async fn voice_task(
    app: tauri::AppHandle,
    state: tauri::State<'_, Core>,
    request: CapabilityRequest,
    audio: Option<String>,
) -> Result<Value, String> {
    let _voice = tokio::time::timeout(Duration::from_secs(60), state.voice_gate.lock())
        .await
        .map_err(|_| "Outra transcrição está em andamento")?;
    let token = state.job(&request.id)?;
    let work = state.root.join("work").join(store::id());
    let result = async {
        fs::create_dir_all(&work).map_err(|_| "Não foi possível preparar tarefa")?;
        let prefs = state.prefs()?;
        if prefs.usage_mode == UsageMode::CloudOnly || request.request_cloud {
            return Err("Áudio não verificado permanece local. Use transcrição local para proteger seu conteúdo.".into());
        }
        let model = state.transcriber(&prefs, false)?;
        let original = if let Some(encoded) = audio {
            if encoded.len() > 90 * 1024 * 1024 {
                return Err("Áudio muito grande".into());
            }
            let path = work.join("recording.wav");
            fs::write(&path, base64::engine::general_purpose::STANDARD.decode(encoded).map_err(|_| "Áudio inválido")?)
                .map_err(|_| "Não foi possível preparar gravação")?;
            path
        } else {
            let selected = tauri::async_runtime::spawn_blocking(move || app.dialog().file().add_filter("Áudio WAV", &["wav"]).blocking_pick_file())
                .await
                .map_err(|_| "Não foi possível escolher áudio")?;
            selected.ok_or("Seleção de áudio cancelada")?.into_path().map_err(|_| "Arquivo inválido")?
        };
        let input = work.join("input.wav");
        let normalized = input.clone();
        tauri::async_runtime::spawn_blocking(move || normalize_wav(&original, &normalized))
            .await
            .map_err(|_| "Falha ao preparar áudio")??;
        let base = work.join("transcript");
        state.runtime.transcribe(&model, &input, &base, &token).await?;
        let text = fs::read_to_string(base.with_extension("txt")).map_err(|_| "A transcrição não produziu texto")?;
        Ok(json!({"text":text}))
    }
    .await;
    let _ = fs::remove_dir_all(work);
    state.finish(&request.id);
    result
}

#[tauri::command]
pub async fn import_legacy(
    app: tauri::AppHandle,
    state: tauri::State<'_, Core>,
) -> Result<Option<usize>, String> {
    let _gate = state
        .runtime
        .gate
        .try_lock()
        .map_err(|_| "Interrompa a tarefa atual antes de importar dados")?;
    let selected = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Banco SQLite", &["db", "sqlite", "sqlite3"])
            .set_title("Importar uma cópia do histórico anterior da Ayla")
            .blocking_pick_file()
    })
    .await
    .map_err(|_| "Não foi possível escolher banco")?;
    let Some(file) = selected else {
        return Ok(None);
    };
    let source = file.into_path().map_err(|_| "Banco inválido")?;
    let backup = state
        .root
        .join("backups")
        .join(format!("before-import-{}.db", store::id()));
    fs::create_dir_all(backup.parent().unwrap()).map_err(|_| "Não foi possível criar backup")?;
    let db = state.store.lock().map_err(|_| "Dados ocupados")?;
    db.backup(&backup)
        .map_err(|_| "Não foi possível preservar o banco atual")?;
    db.import_legacy(&source).map(Some).map_err(|_| {
        "O arquivo não contém um histórico compatível. O banco anterior foi preservado.".into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_second_reply_waits_its_turn_and_can_be_cancelled_while_waiting() {
        let root = std::env::temp_dir().join(format!("ayla-gate-{}", store::id()));
        let core = std::sync::Arc::new(Core::open(root.clone()).unwrap());
        let first = core.take_gate(&CancellationToken::new(), || panic!("livre: não espera")).await.unwrap();
        let cancelled = CancellationToken::new();
        let waiting = std::sync::atomic::AtomicBool::new(false);
        let stop = cancelled.clone();
        let (result, ()) = tokio::join!(core.take_gate(&cancelled, || waiting.store(true, std::sync::atomic::Ordering::SeqCst)), async { stop.cancel(); });
        assert!(result.is_err() && waiting.load(std::sync::atomic::Ordering::SeqCst));
        drop(result);
        let queued = { let core = core.clone(); tokio::spawn(async move { core.take_gate(&CancellationToken::new(), || {}).await.map(|_| ()) }) };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!queued.is_finished());
        drop(first);
        tokio::time::timeout(Duration::from_secs(2), queued).await.unwrap().unwrap().unwrap();
        drop(core);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn athena_compares_distinct_notes_from_the_matching_folder() {
        let root = std::env::temp_dir().join(format!("ayla-athena-core-{}", store::id()));
        let core = Core::open(root.clone()).unwrap();
        let other_project = core.store.lock().unwrap().create_project("Outra mesa", "").unwrap();
        let vault = root.join("wilderfeast");
        let other_vault = root.join("Outra mesa");
        let prefs = Preferences {
            athena_vaults: vec![
                other_vault.to_string_lossy().into_owned(),
                vault.to_string_lossy().into_owned(),
            ],
            ..Preferences::default()
        };
        let write = |base: &std::path::Path, path: &str, content: &str| {
            let file = base.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, content).unwrap();
        };
        write(&root.join("athena"), "NPCs Rápidos.md", "NPC legal para Wilderfeast: um mercador.");
        write(&other_vault, "NPCs/Nera.md", "Uma cartógrafa de montanhas.");
        write(&vault, "Locais/Porto.md", "Um porto movimentado.");
        write(&vault, "NPCs/Vila/Lua.md", "Uma cozinheira que ouve as árvores.");
        write(&vault, "NPCs/Aliados/Ivo.md", &"Um guardião do rio. ".repeat(60));
        let project_notes = root.join("projetos-notas").join(&other_project.id);
        write(&project_notes, "Wilderfeast/NPCs/Segredo.md", "Uma nota privada de outro projeto.");
        core.sync_notes(&project_notes).await.unwrap();

        let cancel = CancellationToken::new();
        let notes = core.notes_context(&prefs, "qual NPC mais legal que eu criei no Wilderfeast?", None, &cancel).await;
        assert_eq!(notes.len(), 2);
        assert!(notes.iter().any(|(name, text)| name == "Lua" && text.contains("cozinheira")));
        assert!(notes.iter().any(|(name, text)| name == "Ivo" && text.contains("guardião")));
        let block = athena::context_block(&notes).unwrap();
        assert!(block.contains("[nota: Lua]") && block.contains("[nota: Ivo]"));
        assert!(!block.contains(vault.to_str().unwrap()));

        let notes = core.notes_context(&prefs, "O que está escrito sobre cartógrafa?", None, &cancel).await;
        assert_eq!(notes[0].0, "Nera");
        drop(core);
        fs::remove_dir_all(root).unwrap();
    }

    /// Rode com AYLA_MODEL_CACHE=/var/tmp/ayla-benchmark: ffmpeg e Whisper vêm do cache (sem rede).
    #[tokio::test]
    #[ignore = "ffmpeg e Whisper reais via cache local; motor do Whisper de /tmp/ayla-p1-runtime-validation"]
    async fn real_attachments_are_read_transcribed_and_seen() {
        let speech = std::path::Path::new("/var/tmp/ayla-benchmark/fleurs/pcm16/7377029495898915434.wav");
        let root = std::env::temp_dir().join(format!("ayla-files-real-{}", store::id()));
        let core = Core::open(root.clone()).unwrap();
        fs::create_dir_all(root.join("runtimes")).unwrap();
        std::os::unix::fs::symlink("/tmp/ayla-p1-runtime-validation/whisper", root.join("runtimes").join("whisper")).unwrap();
        let token = CancellationToken::new();
        let whisper = runtime::install_model(&root.join("models"), models::catalog().iter().find(|e| e.id == "whisper-small").unwrap(), &token, |_| {}).await.unwrap();
        core.store.lock().unwrap().save_model(&whisper.id, &serde_json::to_string(&whisper).unwrap()).unwrap();
        let prefs = core.prefs().unwrap();
        let mut events = Vec::new();
        let start = Instant::now();
        let ffmpeg = core.ffmpeg(&token, &mut |e| events.push(e)).await.unwrap();
        eprintln!("ffmpeg pronto em {} ms", start.elapsed().as_millis());
        let make = |name: &str, args: &[&str]| {
            let out = root.join(name);
            let status = std::process::Command::new(&ffmpeg).args(["-nostdin", "-loglevel", "error"]).args(args).arg("-y").arg(&out).status().unwrap();
            assert!(status.success(), "gerar {name}");
            out
        };
        let speech_arg = speech.to_str().unwrap();
        let video = make("reuniao.mp4", &["-f", "lavfi", "-i", "testsrc=duration=12:size=320x240:rate=10", "-i", speech_arg, "-shortest", "-c:v", "mpeg4", "-c:a", "aac"]);
        let gif = make("anim.gif", &["-f", "lavfi", "-i", "testsrc=duration=2:size=160x120:rate=5"]);
        let mp3 = make("audio.mp3", &["-i", speech_arg, "-c:a", "libmp3lame"]);
        // PDF mínimo com texto em Helvetica, com tabela xref correta.
        let stream = "BT /F1 18 Tf 20 100 Td (Relatorio da Ayla sobre energia solar) Tj ET";
        let objects = ["<< /Type /Catalog /Pages 2 0 R >>".to_string(), "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(), "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 144] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".into(), format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()), "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into()];
        let mut pdf = String::from("%PDF-1.4\n");
        let mut offsets = Vec::new();
        for (i, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.push_str(&format!("{} 0 obj\n{object}\nendobj\n", i + 1));
        }
        let xref = pdf.len();
        pdf.push_str(&format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1));
        for offset in offsets { pdf.push_str(&format!("{offset:010} 00000 n \n")); }
        pdf.push_str(&format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1));
        let pdf_path = root.join("relatorio.pdf");
        fs::write(&pdf_path, pdf).unwrap();

        let add = |path: &std::path::Path| core.store_file(path.file_name().unwrap().to_str().unwrap(), |dest| fs::copy(path, dest).map_err(|e| e.to_string()), None, false).unwrap();
        let (video, gif, mp3, pdf) = (add(&video), add(&gif), add(&mp3), add(&pdf_path));
        assert_eq!((video.kind, gif.kind, mp3.kind, pdf.kind), (Kind::Video, Kind::Gif, Kind::Audio, Kind::Pdf));
        for file in [&video, &gif, &mp3, &pdf] {
            let start = Instant::now();
            let prepared = core.prepare_file(&prefs, file, &token, &mut |e| events.push(e)).await.unwrap();
            eprintln!("{}: {} ms · {} imagens · {:?}", file.name, start.elapsed().as_millis(), prepared.images.len(), prepared.text.as_deref().map(|t| t.chars().take(90).collect::<String>()));
            match file.kind {
                Kind::Video => { assert!(prepared.text.unwrap().to_lowercase().contains("acordo")); assert_eq!(prepared.images.len(), 4); }
                Kind::Gif => assert!(!prepared.images.is_empty() && prepared.text.is_none()),
                Kind::Audio => assert!(prepared.text.unwrap().to_lowercase().contains("presença militar")),
                Kind::Pdf => assert!(prepared.text.unwrap().contains("energia solar")),
                _ => unreachable!(),
            }
        }
        // Segunda vez: tudo do cache, sem ffmpeg nem Whisper.
        let cached = core.store.lock().unwrap().file(&mp3.id).unwrap().unwrap();
        let start = Instant::now();
        assert!(core.prepare_file(&prefs, &cached, &token, &mut |_| {}).await.unwrap().text.is_some());
        assert!(start.elapsed() < Duration::from_millis(200), "transcrição em cache");
        assert!(events.iter().any(|e| e["text"].as_str().is_some_and(|t| t.starts_with("Ouvindo"))));
        drop(core);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn titles_are_short_and_clean() {
        assert_eq!(quick_title("me ajuda a planejar um jantar pra seis pessoas no sábado?"), "Me ajuda a planejar um jantar…");
        assert_eq!(quick_title("  oi!  "), "Oi");
        assert_eq!(quick_title("\n"), DEFAULT_TITLE);
        assert!(quick_title(&"palavraenorme".repeat(10)).ends_with('…'));
        assert_eq!(clean_title(r#"{"titulo":"  \"Jantar de sábado.\" "}"#).as_deref(), Some("Jantar de sábado"));
        assert_eq!(clean_title(r#"{"titulo":"planejamento do TCC"}"#).as_deref(), Some("Planejamento do TCC"));
        assert_eq!(clean_title(r#"{"titulo":""}"#), None);
        assert_eq!(clean_title("não é json"), None);
        assert_eq!(clean_title(r#"{"titulo":"um dois três quatro cinco seis sete oito nove dez onze"}"#), None);
        let path = std::env::temp_dir().join(format!("ayla-title-{}.db", store::id()));
        let db = Store::open(&path).unwrap();
        let c = db.create_conversation(DEFAULT_TITLE, None).unwrap();
        assert!(db.retitle(&c.id, DEFAULT_TITLE, "Jantar").unwrap());
        assert!(!db.retitle(&c.id, DEFAULT_TITLE, "Outro").unwrap(), "não sobrescreve um título já trocado");
        assert_eq!(db.conversation_title(&c.id).unwrap(), "Jantar");
        drop(db);
        for suffix in ["", "-wal", "-shm"] { let _ = std::fs::remove_file(format!("{}{suffix}", path.display())); }
    }
    #[test]
    fn iris_reads_linked_subfolders_in_place_and_revokes_access() {
        let tmp = std::env::temp_dir().join(format!("ayla-iris-folder-{}", store::id()));
        let core = Core::open(tmp.join("ayla")).unwrap();
        let dir = tmp.join("Imagens");
        fs::create_dir_all(dir.join("férias/2026")).unwrap();
        fs::create_dir_all(dir.join("férias/2026/Notas")).unwrap();
        fs::create_dir_all(dir.join("cofre/Mapas")).unwrap();
        for name in ["foto.JPG", "férias/video.mp4", "férias/2026/doc.pdf", "férias/2026/Notas/Resumo.md", "cofre/Mapas/mapa.png", "notas.txt"] {
            fs::write(dir.join(name), b"original").unwrap();
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&core.root, dir.join("fora")).unwrap();
        let hidden = dir.join(".oculta").join("album");
        fs::create_dir_all(&hidden).unwrap();
        fs::write(hidden.join("foto.jpg"), b"oculto").unwrap();
        assert!(linked_media_path(&hidden, &iris_root_id(&hidden), "foto.jpg").is_none());
        assert!(linked_media_path(&dir, &iris_root_id(&dir), ".oculta/album/foto.jpg").is_none());
        let (listed, folders) = athena::scan_attachments(&dir, None);
        assert_eq!(folders, ["cofre", "cofre/Mapas", "férias", "férias/2026"]);
        assert_eq!(listed.len(), 4);
        let id = iris_root_id(&dir);
        let relative = "férias/2026/doc.pdf";
        let mut prefs = core.prefs().unwrap();
        prefs.iris_folder = Some(dir.clone());
        core.preferences.lock().unwrap().save(&prefs).unwrap();
        let request = tauri::http::Request::builder()
            .uri(format!("athena-file://localhost/{id}/f%C3%A9rias%2F2026%2Fdoc.pdf"))
            .body(Vec::new()).unwrap();
        assert_eq!(athena_file(&core, &request).body(), b"original");
        fs::write(dir.join(relative), b"alterado").unwrap();
        assert_eq!(athena_file(&core, &request).body(), b"alterado");
        assert!(linked_media_path(&dir, &id, "../fora.pdf").is_none());
        assert!(linked_media_path(&dir, &id, "notas.txt").is_none());
        #[cfg(unix)] {
            fs::write(core.root.join("outro.pdf"), b"externo").unwrap();
            assert!(linked_media_path(&dir, &id, "fora/outro.pdf").is_none());
        }
        prefs.iris_folder = None;
        core.preferences.lock().unwrap().save(&prefs).unwrap();
        assert_eq!(athena_file(&core, &request).status(), 404);
        assert_eq!(fs::read(dir.join(relative)).unwrap(), b"alterado");
        assert!(core.iris_stored().unwrap().is_empty());
        drop(core);
        fs::remove_dir_all(tmp).unwrap();
    }

    #[test]
    fn iris_lists_sent_generated_and_kept_media_with_their_origin() {
        let root = std::env::temp_dir().join(format!("ayla-iris-{}", store::id()));
        let core = Core::open(root.clone()).unwrap();
        let png = b"\x89PNG\r\n\x1a\n0000000000";
        let add = |name: &str, bytes: &'static [u8], kept: bool| core.store_file(name, |dest| fs::write(dest, bytes).map(|_| bytes.len() as u64).map_err(|e| e.to_string()), None, kept).unwrap();
        let sent = add("foto.png", png, false);
        let generated = add("imagem-1.png", png, false);
        let manual = add("contrato.pdf", b"%PDF-1.4 x", true);
        let pending = add("rascunho.png", png, false);
        let text = add("notas.md", b"# x", true);
        let chat = core.store.lock().unwrap().create_conversation("Viagem", None).unwrap();
        core.store.lock().unwrap().save_turn(&chat.id, "olha", "pronto", true, &[sent.id.clone()], &[generated.id.clone()], None).unwrap();
        let items: Vec<Value> = core.iris_stored().unwrap().into_iter().map(|(_, v)| v).collect();
        let find = |id: &str| items.iter().find(|v| v["file"] == id).cloned();
        assert_eq!((find(&sent.id).unwrap()["source"].clone(), find(&sent.id).unwrap()["place"].clone(), find(&sent.id).unwrap()["kind"].clone()), (json!("chat"), json!("Viagem"), json!("image")));
        assert_eq!(find(&generated.id).unwrap()["source"], "gerada");
        assert_eq!((find(&manual.id).unwrap()["source"].clone(), find(&manual.id).unwrap()["place"].clone(), find(&manual.id).unwrap()["kind"].clone()), (json!("arquivos"), json!("Geral"), json!("pdf")));
        assert!(find(&pending.id).is_none(), "anexo não enviado fica de fora");
        assert!(find(&text.id).is_none(), "texto não é mídia");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn protected_folder_moves_paste_and_banner_stay_encrypted() {
        let root = std::env::temp_dir().join(format!("ayla-protected-{}", store::id()));
        fs::create_dir_all(root.join("Senhas")).unwrap();
        fs::write(root.join("Senhas/Email.md"), "segredo").unwrap();
        fs::write(root.join("Solta.md"), "vai entrar").unwrap();
        fs::write(root.join("foto.png"), b"\x89PNG\r\n\x1a\nfoto").unwrap();
        athena_lock::protect(&root, "Senhas", "senha forte 123").unwrap();
        let place = protected(&root, "Senhas/Email.md").unwrap();
        assert_eq!((place.folder.as_str(), place.inner.as_str()), ("Senhas", "Email.md"));
        assert_eq!(protected(&root, "Senhas").unwrap().inner, "");
        assert!(protected(&root, "Solta.md").is_none());

        // Bloqueada: nada entra, nada sai, e não pode ser apagada (nem ela, nem a pasta de cima).
        assert!(move_across(&root, "Solta.md", "Senhas/Solta.md").is_err());
        assert!(root.join("Solta.md").is_file());
        assert!(trash_folder_in(&root, "Senhas").is_err());
        assert!(trash_folder_in(&root, "Senhas/Pessoal").is_err());
        fs::create_dir_all(root.join("Cofre")).unwrap();
        fs::rename(root.join("Senhas"), root.join("Cofre/Senhas")).unwrap();
        assert!(trash_folder_in(&root, "Cofre").is_err(), "pasta de cima também não");
        assert_eq!(locked_inside(&root).as_deref(), Some("Cofre/Senhas"));
        fs::rename(root.join("Cofre/Senhas"), root.join("Senhas")).unwrap();
        assert!(root.join("Senhas").join(athena_lock::LOCK_FILE).is_file() && !root.join(".trash").exists(), "nada foi apagado");

        let dir = root.join("Senhas");
        athena_lock::start_session(&dir, athena_lock::unlock(&dir, "senha forte 123").unwrap());
        assert_eq!(move_across(&root, "Solta.md", "Senhas/Pessoal/Solta.md").unwrap(), "Senhas/Pessoal/Solta.md");
        assert!(!root.join("Solta.md").exists(), "o original sai do disco");
        assert_eq!(move_across(&root, "foto.png", "Senhas/foto.png").unwrap(), "Senhas/foto.png");
        assert!(move_across(&root, "Senhas/Email.md", "Senhas/Pessoal/Solta.md").is_err(), "não sobrescreve");

        let (embed, stamp) = protected_paste(&protected(&root, "Senhas/Email.md").unwrap(), "segredo ", "", i64::MAX, b"\x89PNG\r\n\x1a\ncolada").unwrap();
        assert!(embed.starts_with("Senhas/Imagens/Email/Imagem-") && embed.ends_with(".png"));
        let (updated, _) = protected_banner(&root, &protected(&root, "Senhas/Email.md").unwrap(), &format!("segredo ![[{embed}]]"), stamp, "Senhas/foto.png").unwrap();
        assert!(updated.starts_with("---\nbanner: Senhas/foto.png\n---"));
        assert!(protected_banner(&root, &protected(&root, "Senhas/Email.md").unwrap(), &updated, i64::MAX, "Senhas/Email.md").is_err(), "só imagens");

        // De volta para fora: decifrado só no destino comum.
        assert_eq!(move_across(&root, "Senhas/Pessoal/Solta.md", "Solta.md").unwrap(), "Solta.md");
        assert_eq!(fs::read_to_string(root.join("Solta.md")).unwrap(), "vai entrar");
        let on_disk: Vec<String> = fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        assert!(on_disk.iter().all(|n| n.ends_with(".athena") || n == athena_lock::LOCK_FILE), "{on_disk:?}");
        assert_eq!(locked_inside(&root), None, "aberta, pode sair");
        athena_lock::end_session(&dir);
        assert!(athena::list(&root).iter().all(|n| !n.path.starts_with("Senhas")));
        assert!(athena::attachments(&root).is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn athena_overview_connects_existing_notes_within_each_root() {
        let tmp = std::env::temp_dir().join(format!("ayla-graph-{}", store::id()));
        let first = tmp.join("primeiro");
        let second = tmp.join("segundo");
        fs::create_dir_all(first.join("pastas")).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::write(first.join("A.md"), "# A\n#aventura [[B#Seção|apelido]] [[B]] [Mapa](<pastas/Mapa de teste.md#Norte>)\n`[[Isolada]]`\n~~~\n[[Isolada]]\n~~~\n[[A]] [[Ausente]]").unwrap();
        fs::write(first.join("B.md"), "[[A]]").unwrap();
        fs::write(first.join("Isolada.md"), "Nota sem relação.").unwrap();
        fs::write(first.join("pastas/Mapa de teste.md"), "").unwrap();
        fs::write(second.join("A.md"), "[[B]]").unwrap();
        let overview = athena_overview_for(vec![
            AthenaRoot { id: "one".into(), kind: "vault", name: "Primeiro".into(), path: first, available: true },
            AthenaRoot { id: "two".into(), kind: "vault", name: "Segundo".into(), path: second, available: true },
        ]);
        assert_eq!(overview["graph"]["nodes"].as_array().unwrap().len(), 5);
        assert_eq!(overview["graph"]["edges"], json!([[0, 1], [0, 3]]));
        assert_eq!(overview["tags"][0]["tag"], "aventura");
        assert_eq!(overview["tags"][0]["notes"][0]["root"], "one");
        fs::remove_dir_all(tmp).unwrap();
    }

    #[test]
    fn iris_groups_project_conversations_and_athena_media_in_the_same_root() {
        let root = std::env::temp_dir().join(format!("ayla-iris-project-{}", store::id()));
        let core = Core::open(root.clone()).unwrap();
        let (first, second, empty, chat) = {
            let db = core.store.lock().unwrap();
            let first = db.create_project("Campanha", "").unwrap();
            let second = db.create_project("Campanha", "").unwrap();
            let empty = db.create_project("Sem mídias", "").unwrap();
            let chat = db.create_conversation("Conversa do projeto", Some(&first.id)).unwrap();
            (first, second, empty, chat)
        };
        let first_root = project_root_id(&first.id);
        let second_root = project_root_id(&second.id);
        let empty_root = project_root_id(&empty.id);
        let prefs = core.prefs().unwrap();
        let project_dir = core.athena_path(&prefs, &first_root).unwrap();
        let folder = athena::mkdir(&project_dir, "", "Mapas").unwrap();
        let png = b"\x89PNG\r\n\x1a\n0000000000";
        fs::write(project_dir.join(&folder).join("mapa.png"), png).unwrap();
        let add = |name: &str, project, kept| core.store_file(name, |dest|
            fs::write(dest, png).map(|_| png.len() as u64).map_err(|e| e.to_string()), project, kept).unwrap();
        // A conversa define o projeto mesmo quando o anexo foi preparado sem essa associação.
        let sent = add("chat.png", None, false);
        let second_file = add("outro.png", Some(second.id.clone()), true);
        core.store.lock().unwrap().save_turn(&chat.id, "olha", "pronto", true, &[sent.id], &[], None).unwrap();

        let library = tauri::async_runtime::block_on(iris_library_for(&core)).unwrap();
        let roots = library["athenaRoots"].as_array().unwrap();
        let get_root = |id: &str| roots.iter().find(|root| root["id"] == id);
        assert_eq!(get_root(&first_root).unwrap()["folders"], json!(["Mapas"]));
        assert_eq!(get_root(&second_root).unwrap()["folders"], json!([]));
        assert!(get_root(&empty_root).is_none());
        let items = library["items"].as_array().unwrap();
        let in_project: Vec<_> = items.iter().filter(|item| item["root"] == first_root).collect();
        assert_eq!(in_project.len(), 2);
        assert!(in_project.iter().any(|item| item["source"] == "chat" && item["name"] == "chat.png"));
        assert!(in_project.iter().any(|item| item["source"] == "athena" && item["path"] == "Mapas/mapa.png"));
        assert!(items.iter().any(|item| item["file"] == second_file.id && item["root"] == second_root));
        drop(core);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn files_are_copied_classified_linked_and_cleaned() {
        let root = std::env::temp_dir().join(format!("ayla-files-core-{}", store::id()));
        let core = Core::open(root.clone()).unwrap();
        let project = core.store.lock().unwrap().create_project("TCC", "").unwrap();
        let note = core.store_file("../notas.md", |dest| fs::write(dest, b"# Plano").map(|_| 7).map_err(|e| e.to_string()), Some(project.id.clone()), false).unwrap();
        assert_eq!((note.name.as_str(), note.kind, note.bytes), ("notas.md", Kind::Text, 7));
        assert_eq!(note.path, format!("files/{}/notas.md", note.id));
        assert!(root.join(&note.path).is_file());
        // Formato não suportado: nada fica no disco nem no banco.
        assert!(core.store_file("app.exe", |dest| fs::write(dest, b"MZ").map(|_| 2).map_err(|e| e.to_string()), None, false).is_err());
        assert_eq!(fs::read_dir(root.join("files")).unwrap().count(), 1);
        assert!(core.store_file("x.md", |dest| fs::write(dest, b"x").map(|_| 1).map_err(|e| e.to_string()), Some("inexistente".into()), false).is_err());
        // Anexo pendente não aparece em Arquivos; depois de enviado, aparece e fica ligado à mensagem.
        assert!(core.store.lock().unwrap().project_files(Some(&project.id)).unwrap().is_empty());
        let chat = core.store.lock().unwrap().create_conversation("c", Some(&project.id)).unwrap();
        core.store.lock().unwrap().save_turn(&chat.id, "resuma", "ok", true, &[note.id.clone()], &[], None).unwrap();
        let listed = core.store.lock().unwrap().project_files(Some(&project.id)).unwrap();
        assert_eq!(listed.len(), 1);
        let messages = core.store.lock().unwrap().messages(&chat.id).unwrap();
        assert_eq!(messages[0].attachments.iter().map(|f| f.id.clone()).collect::<Vec<_>>(), vec![note.id.clone()]);
        assert!(messages[1].attachments.is_empty());
        // Cópia sem registro some ao abrir; a registrada fica.
        fs::create_dir_all(root.join("files").join("orfa")).unwrap();
        drop(core);
        let core = Core::open(root.clone()).unwrap();
        assert!(!root.join("files").join("orfa").exists() && root.join(&note.path).is_file());
        // Apagar o projeto apaga a cópia.
        let copies = core.store.lock().unwrap().project_file_paths(&project.id).unwrap();
        assert!(core.store.lock().unwrap().delete_project(&project.id).unwrap());
        for path in copies {
            remove_copy(&core.root, &path);
        }
        assert!(!root.join(&note.path).exists());
        remove_copy(&core.root, "../fora/arquivo");
        drop(core);
        fs::remove_dir_all(root).unwrap();
    }
    use std::path::Path;

    #[test]
    fn failed_extraction_keeps_later_messages_pending() {
        let root = std::env::temp_dir().join(format!("ayla-memory-cursor-{}", store::id()));
        let core = Core::open(root.clone()).unwrap();
        let prefs = core.prefs().unwrap();
        let (conversation, until) = {
            let db = core.store.lock().unwrap();
            let id = db.create_conversation("Teste", None).unwrap().id;
            db.add_message(&id, "user", "Primeiro turno", true).unwrap();
            let (_, until) = db.extraction_range(&id).unwrap().unwrap();
            db.add_message(&id, "user", "Turno posterior", true).unwrap();
            (id, until)
        };
        core.advance_failed_extraction(&prefs, &conversation, until);
        let path = root.join("teste.gguf");
        fs::write(&path, b"modelo de teste").unwrap();
        {
            let db = core.store.lock().unwrap();
            assert_eq!(db.extraction_range(&conversation).unwrap().unwrap().0, 0);
            let model = json!({"id":"teste","name":"Teste","capability":"text","bytes":0,"memory_bytes":0,"path":path,"sha256":""});
            db.save_model("teste", &model.to_string()).unwrap();
        }
        core.advance_failed_extraction(&prefs, &conversation, until);
        let (cursor, last) = core.store.lock().unwrap().extraction_range(&conversation).unwrap().unwrap();
        assert_eq!(cursor, until);
        assert!(last > cursor);
        drop(core);
        fs::remove_dir_all(root).unwrap();
    }

    /// Modelos reais já baixados em /var/tmp/ayla-benchmark; dados em pasta temporária.
    #[tokio::test]
    #[ignore = "usa modelos reais locais (Gemma 4 E4B + EmbeddingGemma); sem downloads"]
    async fn real_memory_extraction_recall_update_and_forget() {
        let bench = Path::new("/var/tmp/ayla-benchmark");
        let root = std::env::temp_dir().join(format!("ayla-memory-real-{}", store::id()));
        fs::create_dir_all(root.join("runtimes")).unwrap();
        for id in ["llama-vulkan", "llama-cpu"] {
            std::os::unix::fs::symlink(format!("/tmp/ayla-p1-runtime-validation/{id}"), root.join("runtimes").join(id)).unwrap();
        }
        let core = Core::open(root.clone()).unwrap();
        let entry = |id: &str| models::catalog().iter().find(|e| e.id == id).unwrap();
        let chat = Model { id: "gemma".into(), name: "Gemma".into(), capability: Capability::Text, capabilities: entry("gemma-4-e4b").capabilities.to_vec(), path: bench.join("gemma-4-E4B_q4_0-it.gguf"), companion: Some(bench.join("gemma-4-E4B-it-mmproj.gguf")), bytes: 1, sha256: String::new(), catalog: Some("gemma-4-e4b".into()), memory_bytes: Some(entry("gemma-4-e4b").memory_bytes()) };
        let embed = Model { id: "emb".into(), name: "Emb".into(), capability: Capability::Embedding, capabilities: vec![], path: bench.join("embeddinggemma-300M-qat-Q4_0.gguf"), companion: None, bytes: 1, sha256: String::new(), catalog: Some("embeddinggemma-300m".into()), memory_bytes: Some(entry("embeddinggemma-300m").memory_bytes()) };
        {
            let db = core.store.lock().unwrap();
            for m in [&chat, &embed] {
                db.save_model(&m.id, &serde_json::to_string(m).unwrap()).unwrap();
            }
        }
        let prefs = core.prefs().unwrap();
        let token = CancellationToken::new();
        let talk = |lines: &[(&str, &str)]| {
            let db = core.store.lock().unwrap();
            let c = db.create_conversation("teste", None).unwrap();
            for (role, text) in lines {
                db.add_message(&c.id, role, text, true).unwrap();
            }
            c.id
        };
        let show = |label: &str, facts: &[Fact]| eprintln!("{label}: {:?}", facts.iter().map(|f| (&f.content, f.privacy)).collect::<Vec<_>>());

        let start = Instant::now();
        let first = talk(&[
            ("user", "Oi! Eu moro em Fortaleza e tenho uma gata chamada Mel."),
            ("assistant", "Que legal! A Mel deve ser uma ótima companhia."),
            ("user", "Prefiro respostas curtas, sem enrolação. Me ajuda a escolher um nome para um café?"),
            ("assistant", "Claro: Grão Nobre, Café Brisa ou Ponto do Grão."),
        ]);
        let learned = core.remember(&prefs, &first, "auto", &token).await.unwrap();
        show(&format!("extração 1 ({} ms)", start.elapsed().as_millis()), &learned);
        assert!(learned.len() >= 2, "deve lembrar fatos duradouros");
        assert!(!learned.iter().any(|f| f.content.to_lowercase().contains("grão")), "não guarda pedidos pontuais");

        let start = Instant::now();
        let recalled = core.recall(&prefs, "Qual o nome do meu pet?", None, &token).await;
        show(&format!("busca pet ({} ms)", start.elapsed().as_millis()), &recalled);
        assert!(recalled.iter().any(|f| f.content.contains("Mel")));
        let unrelated = core.recall(&prefs, "Quanto é 17 vezes 23?", None, &token).await;
        show("busca sem relação", &unrelated);

        let second = talk(&[("user", "Novidade: me mudei para Recife semana passada."), ("assistant", "Que mudança! Boa adaptação em Recife.")]);
        let updated = core.remember(&prefs, &second, "auto", &token).await.unwrap();
        show("atualização", &updated);
        let all = core.store.lock().unwrap().facts(None).unwrap();
        show("todas", &all);
        assert!(all.iter().any(|f| f.content.contains("Recife")));
        assert!(!all.iter().any(|f| f.content.contains("Fortaleza")), "a cidade antiga deve ser substituída");

        let third = talk(&[("user", "Pode esquecer a informação sobre a minha gata."), ("assistant", "Tudo bem, esqueci.")]);
        core.remember(&prefs, &third, "explicit", &token).await.unwrap();
        let all = core.store.lock().unwrap().facts(None).unwrap();
        show("após esquecer", &all);
        assert!(!all.iter().any(|f| f.content.contains("Mel")));

        let fourth = talk(&[("user", "Qual a capital da Austrália?"), ("assistant", "Canberra.")]);
        let nothing = core.remember(&prefs, &fourth, "auto", &token).await.unwrap();
        show("pergunta geral", &nothing);
        assert!(nothing.is_empty());
        eprintln!("RSS workers: {} MB; status {}", core.runtime.resident_bytes() >> 20, core.runtime.status());
        core.runtime.stop_all();
        drop(core);
        fs::remove_dir_all(root).unwrap();
    }
}
