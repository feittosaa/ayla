use super::{
    cloud::{self, Api, Login, Secret},
    files::{self, Kind},
    hardware,
    memory::{self, Fact, Privacy},
    models::{self, Capability, Model},
    notify,
    policy::{self, Sensitivity, UsageMode},
    preferences::{PreferenceStore, Preferences},
    runtime::{self, Runtime},
    store::{self, FileInfo, Scope, Store},
};
use base64::Engine;
use serde::Deserialize;
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
    fn budget(&self, prefs: &Preferences) -> u64 {
        let mut hw = hardware::detect();
        hw.ram_available_bytes = hw
            .ram_available_bytes
            .saturating_add(self.runtime.resident_bytes())
            .min(hw.ram_total_bytes);
        models::memory_budget(&hw, prefs)
    }
    fn models(&self) -> Result<Vec<Model>, String> {
        self.store
            .lock()
            .map_err(|_| "Dados ocupados")?
            .models()
            .map_err(|_| "Não foi possível consultar modelos")?
            .into_iter()
            .map(|raw| serde_json::from_str(&raw).map_err(|_| "Registro de modelo inválido".into()))
            .collect()
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
            let extraction = memory::parse_extraction(&raw, &related)?;
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
#[tauri::command]
pub fn save_preferences(
    state: tauri::State<'_, Core>,
    preferences: Preferences,
) -> Result<(), String> {
    state
        .preferences
        .lock()
        .map_err(|_| "Configuração ocupada")?
        .save(&preferences)
        .map_err(|_| {
            "Não foi possível salvar: verifique os limites e a URL HTTPS do provedor".into()
        })
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
    let db = state.store.lock().map_err(|_| "Dados ocupados")?;
    let copies = db.project_file_paths(&id).unwrap_or_default();
    match db.delete_project(&id) {
        Ok(true) => {
            for path in copies {
                remove_copy(&state.root, &path);
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
    /// Arquivos anexados (ids devolvidos por `add_file`), no máximo 5.
    #[serde(default)]
    pub attachments: Vec<String>,
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
    // A extração em segundo plano cede a vez: cancela e espera liberar.
    let background = state.background.lock().ok().and_then(|mut b| b.take());
    let _gate = match state.runtime.gate.try_lock() {
        Ok(gate) => gate,
        Err(_) => match background {
            Some(task) => {
                task.cancel();
                tokio::time::timeout(Duration::from_secs(10), state.runtime.gate.lock())
                    .await
                    .map_err(|_| "Já existe uma tarefa de IA em andamento")?
            }
            None => return Err("Já existe uma tarefa de IA em andamento".into()),
        },
    };
    let token = state.job(&request.id)?;
    let start = Instant::now();
    let mut partial = String::new();
    let mut first = None;
    let mut chunks = 0;
    let mut metric_model = String::from("unresolved");
    let mut recalled = 0usize;
    let mut result=async{
        let prefs=state.prefs()?;
        let (mut context,consent)={let db=state.store.lock().map_err(|_|"Dados ocupados")?;(db.context(&request.conversation,prefs.history_messages,prefs.history_chars).map_err(|_|"Falha ao consultar conversa")?,db.consent(&request.conversation).map_err(|_|"Conversa não encontrada")?)};
        let gate=policy::cloud_gate(prefs.usage_mode,request.sensitivity,request.reviewed,consent,request.request_cloud,&format!("{}\n{}",prefs.personality,request.message));
        let use_cloud=prefs.usage_mode==UsageMode::CloudOnly||request.request_cloud;
        // Finanças: a Ayla consulta o resumo da Moneta. Esses dados ficam neste computador em
        // qualquer modo, então a resposta é local (com modelo, ou direta, sem modelo).
        let finance={use tauri::Manager;let organ=app.state::<super::organs::Organ>();
            if organ.installed()&&super::organs::finance_question(&request.message){Some(organ.finance().await)}else{None}};
        let use_cloud=use_cloud&&finance.is_none();
        if let Some(data)=&finance{let _=events.send(json!({"type":"routing","text":match data{Ok(_)=>"Moneta consultada · resposta neste computador".to_string(),Err(e)=>format!("Moneta indisponível: {e}")}}));}
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
        // Arquivos de mensagens anteriores continuam disponíveis para perguntas seguintes.
        if !use_cloud{
            let earlier=state.store.lock().map_err(|_|"Dados ocupados")?.conversation_files(&request.conversation).unwrap_or_default();
            for file in earlier{if let Some(text)=file.extracted{if !text.trim().is_empty(){texts.push((file.name,file.kind,text));}}}
        }
        let lengths:Vec<usize>=texts.iter().map(|(_,_,t)|t.chars().count()).collect();
        let blocks:Vec<String>=texts.iter().zip(files::budgets(&lengths,files::PROMPT_BUDGET)).filter(|(_,limit)|*limit>0).map(|((name,kind,text),limit)|files::prompt_block(name,*kind,text,limit)).collect();
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
        if !use_cloud{
            let project=state.store.lock().map_err(|_|"Dados ocupados")?.conversation_project(&request.conversation).ok().flatten().and_then(|id|state.store.lock().ok()?.project(&id).ok().flatten());
            if let Some(p)=&project{system.push_str(&project_block(p));}
            let facts=state.recall(&prefs,&request.message,project.as_ref().map(|p|p.id.as_str()),&token).await;recalled=facts.len();if let Some(block)=memory::context_block(&facts){system.push_str(&block);}
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
            if !gate.allowed{return Err(gate.reason);}
            if request.personality_preview.as_deref()!=Some(prefs.system_prompt()){return Err("A personalidade mudou. Revise novamente antes de enviar à nuvem.".into());}
            if prefs.cloud_model.is_empty(){return Err("Escolha o modelo da nuvem em Configurações › Nuvem".into());}
            // Nunca enviar histórico, memórias, anexos ou dados de órgãos.
            let platform=prefs.cloud_platform().ok_or("Escolha uma plataforma em Configurações › Nuvem")?.to_string();
            let (api,base,secret)=state.cloud_secret(&prefs,&platform).await?;
            let outgoing=cloud::chat_request(api,&base,&secret,&prefs.cloud_model,prefs.system_prompt(),&request.message)?;
            cloud::stream(outgoing,api,&token,&mut report).await?;
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
                    )
                    .is_err()
                {
                    result = Err(
                        "A resposta foi recebida, mas não foi possível salvar esta conversa".into(),
                    );
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
    if result.is_ok() && memory::explicit_command(&request.message) {
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
        db.save_turn(&request.conversation, &prompt, &reply, true, &[], &[file.id.clone()]).map_err(|_| "A imagem foi criada, mas não foi possível salvar a conversa")?;
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
        let db = Store::open(&std::env::temp_dir().join(format!("ayla-title-{}.db", store::id()))).unwrap();
        let c = db.create_conversation(DEFAULT_TITLE, None).unwrap();
        assert!(db.retitle(&c.id, DEFAULT_TITLE, "Jantar").unwrap());
        assert!(!db.retitle(&c.id, DEFAULT_TITLE, "Outro").unwrap(), "não sobrescreve um título já trocado");
        assert_eq!(db.conversation_title(&c.id).unwrap(), "Jantar");
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
        core.store.lock().unwrap().save_turn(&chat.id, "resuma", "ok", true, &[note.id.clone()], &[]).unwrap();
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
