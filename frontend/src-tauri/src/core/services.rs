use super::{
    hardware,
    memory::{self, Fact, Privacy},
    models::{self, Capability, Model},
    policy::{self, Sensitivity, UsageMode},
    preferences::{PreferenceStore, Preferences},
    runtime::{self, Runtime},
    store::{self, Store},
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
    jobs: Mutex<HashMap<String, CancellationToken>>,
    keys: Mutex<HashMap<String, String>>,
    /// Última atividade da conversa; a extração automática espera a pausa.
    last_activity: Mutex<Instant>,
    /// Extração em segundo plano, cancelada quando o usuário volta a conversar.
    background: Mutex<Option<CancellationToken>>,
    /// Estado do preparo automático, exibido pela interface.
    setup: Mutex<Value>,
}
impl Core {
    pub fn open(root: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&root).map_err(|_| "Não foi possível abrir seus dados")?;
        let store = Store::open(&root.join("ayla.db"))
            .map_err(|_| "Não foi possível abrir o banco da Ayla")?;
        Ok(Self {
            preferences: Mutex::new(PreferenceStore::new(&root)),
            store: Mutex::new(store),
            runtime: Arc::new(Runtime::new(root.join("runtimes"))),
            root,
            install_gate: tokio::sync::Mutex::new(()),
            jobs: Mutex::new(HashMap::new()),
            keys: Mutex::new(HashMap::new()),
            last_activity: Mutex::new(Instant::now()),
            background: Mutex::new(None),
            setup: Mutex::new(json!({"active":false})),
        })
    }
    fn prefs(&self) -> Result<Preferences, String> {
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
    fn search(&self, model_id: &str, vector: &[f32], text: &str, minimum: f64, limit: usize) -> Result<Vec<(Fact, Option<f64>)>, String> {
        let db = self.store.lock().map_err(|_| "Dados ocupados")?;
        let mut hits: Vec<(String, f64)> = db.vectors(model_id).map_err(|_| "Falha ao consultar memórias")?
            .into_iter()
            .filter_map(|(id, v)| memory::cosine(vector, &v).filter(|s| *s >= minimum).map(|s| (id, s)))
            .collect();
        hits.sort_by(|a, b| b.1.total_cmp(&a.1));
        hits.truncate(20);
        let words = memory::fts_query(text).map(|q| db.search_text(&q, 20).unwrap_or_default()).unwrap_or_default();
        let ids = memory::fuse(&hits, &words, limit);
        let facts = db.facts_by_ids(&ids).map_err(|_| "Falha ao consultar memórias")?;
        Ok(facts.into_iter().map(|f| { let score = hits.iter().find(|(i, _)| *i == f.id).map(|(_, s)| *s); (f, score) }).collect())
    }
    /// Memórias relevantes ao pedido, para respostas locais. Falhas não impedem a conversa.
    async fn recall(&self, prefs: &Preferences, text: &str, cancel: &CancellationToken) -> Vec<Fact> {
        let Some(model) = self.memory_model(prefs) else { return Vec::new() };
        let result = async {
            self.backfill(prefs, &model, cancel).await?;
            let vector = self.runtime.embed(&model, prefs, &memory::query_text(text), cancel).await?;
            self.search(&model.id, &vector, text, memory::RETRIEVAL_MIN, memory::CONTEXT_LIMIT)
        }
        .await;
        result.map(|items| items.into_iter().map(|(f, _)| f).collect()).unwrap_or_default()
    }
    /// Extrai memórias das mensagens novas de uma conversa e as aplica.
    async fn remember(&self, prefs: &Preferences, conversation: &str, origin: &str, cancel: &CancellationToken) -> Result<Vec<Fact>, String> {
        let range = self.store.lock().map_err(|_| "Dados ocupados")?.extraction_range(conversation).map_err(|_| "Falha ao consultar conversa")?;
        let Some((after, until)) = range else { return Ok(Vec::new()) };
        let embedder = self.memory_model(prefs).ok_or("Ative a memória para lembrar automaticamente")?;
        let models = self.models()?;
        let chat = models::choose(&models, Capability::Text, self.budget(prefs), prefs.context_tokens, None)?;
        let turns = self.store.lock().map_err(|_| "Dados ocupados")?.turns_between(conversation, after, until, 20).map_err(|_| "Falha ao consultar conversa")?;
        if turns.iter().any(|t| t.role == "user") {
            self.backfill(prefs, &embedder, cancel).await?;
            let said: String = turns.iter().filter(|t| t.role == "user").map(|t| t.content.as_str()).collect::<Vec<_>>().join("\n").chars().take(2000).collect();
            let probe = self.runtime.embed(&embedder, prefs, &memory::document_text(&said), cancel).await?;
            let related: Vec<Fact> = self.search(&embedder.id, &probe, &said, memory::RELATED_MIN, 10)?.into_iter().map(|(f, _)| f).collect();
            let raw = self.runtime.complete_json(&chat, prefs, memory::extraction_request(&turns, &related), memory::extraction_schema(), cancel).await?;
            let extraction = memory::parse_extraction(&raw, &related)?;
            let texts: Vec<String> = extraction.remember.iter().map(|f| memory::document_text(&f.content)).collect();
            let vectors = self.runtime.embed_many(&embedder, prefs, &texts, cancel).await?;
            if cancel.is_cancelled() {
                return Err("Tarefa interrompida".into());
            }
            let db = self.store.lock().map_err(|_| "Dados ocupados")?;
            let existing = db.vectors(&embedder.id).map_err(|_| "Falha ao consultar memórias")?;
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
                            db.insert_fact(&new.content, &new.kind, new.privacy, origin, Some(conversation)).ok().map(|f| f.id)
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
    let metrics = state
        .store
        .lock()
        .map_err(|_| "Dados ocupados")?
        .metrics()
        .map_err(|_| "Não foi possível ler métricas")?;
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
        json!({"system_prompt":prefs.system_prompt(),"preferences":prefs,"memory_budget_bytes":budget,"hardware":hw,"models":installed,"runtimes":artifacts,"worker":state.runtime.status(),"metrics":metrics,"recommended_profile":recommended,"catalog":catalog,"recommended_model":recommended_model,"setup":state.setup.lock().map(|s|s.clone()).unwrap_or(Value::Null)}),
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
    let selected = tauri::async_runtime::spawn_blocking(move || {
        if capability == Capability::Speech {
            app.dialog().file().blocking_pick_folder()
        } else {
            app.dialog().file().blocking_pick_file()
        }
    })
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
        if prefs.memory_auto && !installed.iter().any(|m| m.supports(Capability::Embedding)) {
            plan.extend(models::catalog().iter().filter(|e| e.capabilities.contains(&Capability::Embedding)).take(1));
        }
        Ok(plan)
    }
    /// Instala as entradas na ordem, com o llama.cpp antes se faltar; progresso acumulado.
    async fn install_entries(&self, entries: &[&'static models::CatalogEntry], token: &CancellationToken, mut progress: impl FnMut(&str, u64, u64)) -> Result<(), String> {
        let runtimes = self.root.join("runtimes");
        let llama = runtime::artifacts().into_iter().find(|a| a.id == runtime::preferred_llama()).ok_or("Componente desconhecido")?;
        let runtime_bytes = if runtime::llama_installed(&runtimes) { 0 } else { llama.bytes };
        let total = runtime_bytes + entries.iter().map(|e| e.download_bytes()).sum::<u64>();
        if runtime_bytes > 0 {
            runtime::install(&runtimes, llama.id, token, |event| {
                progress("Motor de IA", event["received"].as_u64().unwrap_or(0), total)
            })
            .await?;
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
        let mut done = runtime_bytes;
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
    pub async fn prepare_defaults(&self) -> Result<(), String> {
        let prefs = self.prefs()?;
        if prefs.usage_mode == UsageMode::CloudOnly {
            return Ok(());
        }
        let plan = self.default_plan(&prefs)?;
        if plan.is_empty() {
            return Ok(());
        }
        let Ok(_install) = self.install_gate.try_lock() else { return Ok(()) };
        let token = self.job(SETUP_JOB)?;
        self.set_setup(json!({"active":true,"label":"Preparando","received":0,"total":0}));
        let result = self.install_entries(&plan, &token, |label, received, total| {
            self.set_setup(json!({"active":true,"label":label,"received":received,"total":total}));
        }).await;
        self.finish(SETUP_JOB);
        self.set_setup(match &result {
            Ok(()) => json!({"active":false}),
            Err(_) if token.is_cancelled() => json!({"active":false,"cancelled":true}),
            Err(error) => json!({"active":false,"error":error}),
        });
        result
    }
}

#[tauri::command]
pub async fn setup_start(state: tauri::State<'_, Core>) -> Result<(), String> {
    state.prepare_defaults().await
}

#[tauri::command]
pub async fn install_catalog_model(
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
pub fn conversations(state: tauri::State<'_, Core>) -> Result<Value, String> {
    Ok(json!(state
        .store
        .lock()
        .map_err(|_| "Dados ocupados")?
        .conversations()
        .map_err(|_| "Falha ao ler conversas")?))
}
#[tauri::command]
pub fn create_conversation(state: tauri::State<'_, Core>) -> Result<Value, String> {
    Ok(json!(state
        .store
        .lock()
        .map_err(|_| "Dados ocupados")?
        .create_conversation("Nova conversa")
        .map_err(|_| "Falha ao criar conversa")?))
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
    pub image: Option<String>,
}
#[tauri::command]
pub async fn chat(
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
        let models=state.models()?;let mut kind=if request.image.is_some(){Capability::Vision}else{request.capability.unwrap_or_else(||models::request_kind(&request.message,false))};
        if request.capability.is_none() && kind==Capability::Reasoning && !models.iter().any(|m|m.supports(kind)){kind=Capability::Text;}
        if request.image.is_none(){if let Some(selected)=request.model.as_ref().and_then(|id|models.iter().find(|m|m.id==*id)){
            if !selected.supports(kind)&&matches!(selected.capability,Capability::Text|Capability::Reasoning|Capability::Vision){kind=selected.capability;}
        }}
        let mut selection=models::choose(&models,kind,state.budget(&prefs),prefs.context_tokens,request.model.as_deref());
        if selection.is_err() && request.capability.is_none() && request.model.is_none() && kind==Capability::Reasoning {
            kind=Capability::Text;selection=models::choose(&models,kind,state.budget(&prefs),prefs.context_tokens,None);
        }
        let mut content=json!(request.message);
        if let Some(path)=&request.image{
            if use_cloud{return Err("Anexos permanecem locais nesta versão; o envio não pode ser verificado automaticamente".into());}
            if fs::metadata(path).map_err(|_|"Não foi possível ler imagem")?.len()>20*1024*1024{return Err("Imagem excede o limite de processamento".into());}
            let bytes=fs::read(path).map_err(|_|"Não foi possível ler imagem")?;
            let mime=if bytes.starts_with(b"\x89PNG"){"image/png"}else if bytes.starts_with(&[0xff,0xd8]){"image/jpeg"}else{return Err("Use uma imagem PNG ou JPEG".into());};
            content=json!([{"type":"text","text":request.message},{"type":"image_url","image_url":{"url":format!("data:{mime};base64,{}",base64::engine::general_purpose::STANDARD.encode(bytes))}}]);
        }
        // Memórias locais só entram em respostas locais nesta etapa.
        let mut system=prefs.system_prompt().to_string();
        if !use_cloud{let facts=state.recall(&prefs,&request.message,&token).await;recalled=facts.len();if let Some(block)=memory::context_block(&facts){system.push_str(&block);}}
        context.insert(0,json!({"role":"system","content":system}));
        context.push(json!({"role":"user","content":content}));
        let mut report=|event:Value|{if let Some(text)=event["text"].as_str(){if first.is_none(){first=Some(start.elapsed().as_millis() as u64);}partial.push_str(text);chunks+=1;}if events.send(event).is_err(){token.cancel();}};
        if use_cloud{
            metric_model = prefs.cloud_model.clone();
            if !gate.allowed{return Err(gate.reason);}
            if request.personality_preview.as_deref()!=Some(prefs.system_prompt()){return Err("A personalidade mudou. Revise novamente antes de enviar à nuvem.".into());}
            if prefs.cloud_endpoint.is_empty()||prefs.cloud_model.is_empty(){return Err("Configure um provedor e modelo cloud antes de enviar".into());}
            let key=state.key()?;
            // Nunca enviar histórico, memórias, anexos ou dados de órgãos.
            let client=reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(10)).read_timeout(std::time::Duration::from_secs(90)).redirect(reqwest::redirect::Policy::none()).build().map_err(|_|"Falha no cliente cloud")?;
            let request=client.post(format!("{}/chat/completions",prefs.cloud_endpoint.trim_end_matches('/'))).bearer_auth(key).json(&json!({"model":prefs.cloud_model,"messages":[{"role":"system","content":prefs.system_prompt()},{"role":"user","content":request.message}],"stream":true}));
            runtime::stream_chat(request,&token,&mut report).await?;
        }else{
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
    state.finish(&request.id);
    result
}

#[tauri::command]
pub async fn pick_image(app: tauri::AppHandle) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Imagens", &["png", "jpg", "jpeg"])
            .blocking_pick_file()
            .and_then(|p| p.into_path().ok())
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .map_err(|_| "Não foi possível escolher imagem".into())
}
#[tauri::command]
pub fn backup_data(state: tauri::State<'_, Core>) -> Result<(), String> {
    let root = state.root.join("backups");
    fs::create_dir_all(&root).map_err(|_| "Não foi possível criar backup")?;
    state
        .store
        .lock()
        .map_err(|_| "Dados ocupados")?
        .backup(&root.join(format!("ayla-{}.db", store::id())))
        .map_err(|_| "Não foi possível criar backup".into())
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
            let fact = db.insert_fact(request.text.trim(), "nota", Privacy::Uncertain, "manual", None).map_err(|_| "Não foi possível salvar memória")?;
            db.set_vector(&fact.id, &model_id, &vector).map_err(|_| "Não foi possível salvar memória")?;
            return Ok(json!({"saved":true}));
        }
        let results: Vec<Value> = state.search(&model_id, &vector, &request.text, 0.0, 5)?
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
pub fn memory_list(state: tauri::State<'_, Core>) -> Result<Value, String> {
    let prefs = state.prefs()?;
    let facts = state.store.lock().map_err(|_| "Dados ocupados")?.facts().map_err(|_| "Falha ao consultar memórias")?;
    Ok(json!({"facts":facts,"enabled":prefs.memory_auto,"ready":state.memory_model(&prefs).is_some()}))
}
#[tauri::command]
pub fn conversation_memories(state: tauri::State<'_, Core>, id: String) -> Result<Vec<Fact>, String> {
    state.store.lock().map_err(|_| "Dados ocupados")?.conversation_facts(&id).map_err(|_| "Falha ao consultar memórias".into())
}
/// Cria (sem `id`) ou edita uma memória pelo painel. O vetor é recalculado depois, em segundo plano.
#[tauri::command]
pub fn memory_save(state: tauri::State<'_, Core>, id: Option<String>, content: String, privacy: Privacy) -> Result<(), String> {
    let content = content.trim();
    if content.chars().count() < 3 || content.chars().count() > 1000 {
        return Err("A memória deve ter entre 3 e 1000 caracteres".into());
    }
    let db = state.store.lock().map_err(|_| "Dados ocupados")?;
    match id {
        Some(id) => db.update_fact(&id, Some(content), Some(privacy)).map_err(|_| "Memória não encontrada".to_string()),
        None => db.insert_fact(content, "fato", privacy, "manual", None).map(|_| ()).map_err(|_| "Não foi possível salvar memória".to_string()),
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
    if !prefs.memory_auto || !idle || state.memory_model(&prefs).is_none() {
        return;
    }
    let Ok(_gate) = state.runtime.gate.try_lock() else { return };
    let pending = state.store.lock().ok().and_then(|db| db.pending_extraction().ok().flatten());
    let Some((conversation, _, _)) = pending else { return };
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
        Err(_) => {
            if let Ok(db) = state.store.lock() {
                if let Ok(Some((_, until))) = db.extraction_range(&conversation) {
                    if models::choose(&state.models().unwrap_or_default(), Capability::Text, state.budget(&prefs), prefs.context_tokens, None).is_ok() {
                        let _ = db.set_memory_cursor(&conversation, until);
                    }
                }
            }
        }
    }
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

#[tauri::command]
pub async fn voice_task(
    app: tauri::AppHandle,
    state: tauri::State<'_, Core>,
    request: CapabilityRequest,
    transcribe: bool,
    audio: Option<String>,
    voice: Option<String>,
) -> Result<Value, String> {
    let _gate = state
        .runtime
        .gate
        .try_lock()
        .map_err(|_| "Já existe uma tarefa em andamento")?;
    let token = state.job(&request.id)?;
    let work = state.root.join("work").join(store::id());
    let result=async{
        fs::create_dir_all(&work).map_err(|_|"Não foi possível preparar tarefa")?;
        let prefs=state.prefs()?;let use_cloud=prefs.usage_mode==UsageMode::CloudOnly||request.request_cloud;
        if transcribe && use_cloud{return Err("Áudio não verificado permanece local. Use transcrição local para proteger seu conteúdo.".into());}
        if !transcribe && (request.text.is_empty()||request.text.len()>20000){return Err("Informe um texto de até vinte mil caracteres".into());}
        let output=work.join("speech.wav");
        if use_cloud{
            let gate=policy::cloud_gate(prefs.usage_mode,request.sensitivity,request.reviewed,true,request.request_cloud,&request.text);if !gate.allowed{return Err(gate.reason);}
            let model=request.cloud_model.as_deref().filter(|s|!s.is_empty()).unwrap_or(&prefs.cloud_model);
            let voice=voice.filter(|s|!s.is_empty()).ok_or("Informe a voz disponibilizada pelo seu provedor")?;
            let client=reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(std::time::Duration::from_secs(90)).build().map_err(|_|"Falha no cliente cloud")?;
            let response=tokio::select!{_=token.cancelled()=>return Err("Tarefa interrompida".into()),r=client.post(format!("{}/audio/speech",prefs.cloud_endpoint.trim_end_matches('/'))).bearer_auth(state.key()?).json(&json!({"model":model,"input":request.text,"voice":voice,"response_format":"wav"})).send()=>r.map_err(|_|"Falha no provedor de voz")?};
            let response=response.error_for_status().map_err(|_|"Provedor recusou a síntese de voz")?;let mut bytes=Vec::new();let mut stream=response.bytes_stream();
            use futures_util::StreamExt;while let Some(chunk)=tokio::select!{_=token.cancelled()=>return Err("Tarefa interrompida".into()),c=stream.next()=>c}{bytes.extend(chunk.map_err(|_|"Áudio interrompido")?);if bytes.len()>64*1024*1024{return Err("Áudio excede limite de processamento".into());}}
            return Ok(json!({"audio":base64::engine::general_purpose::STANDARD.encode(bytes)}));
        }
        let kind=if transcribe{Capability::Transcription}else{Capability::Speech};let model=models::choose(&state.models()?,kind,state.budget(&prefs),prefs.context_tokens,None)?;
        let input=work.join("input.wav");
        if transcribe{
            let original=if let Some(encoded)=audio{
                if encoded.len()>90*1024*1024{return Err("Áudio muito grande".into());}
                let path=work.join("recording.wav");fs::write(&path,base64::engine::general_purpose::STANDARD.decode(encoded).map_err(|_|"Áudio inválido")?).map_err(|_|"Não foi possível preparar gravação")?;path
            }else{
                let selected=tauri::async_runtime::spawn_blocking(move||app.dialog().file().add_filter("Áudio WAV",&["wav"]).blocking_pick_file()).await.map_err(|_|"Não foi possível escolher áudio")?;
                selected.ok_or("Seleção de áudio cancelada")?.into_path().map_err(|_|"Arquivo inválido")?
            };
            let normalized=input.clone();tauri::async_runtime::spawn_blocking(move||normalize_wav(&original,&normalized)).await.map_err(|_|"Falha ao preparar áudio")??;
            let base=work.join("transcript");state.runtime.voice(&model,&input,"",&base,&token).await?;
            let text=fs::read_to_string(base.with_extension("txt")).map_err(|_|"A transcrição não produziu texto")?;Ok(json!({"text":text}))
        }else{
            state.runtime.voice(&model,&input,&request.text,&output,&token).await?;
            if fs::metadata(&output).map_err(|_|"A voz não produziu áudio")?.len()>64*1024*1024{return Err("Áudio excede limite de processamento".into());}
            let bytes=fs::read(output).map_err(|_|"Não foi possível ler o áudio")?;Ok(json!({"audio":base64::engine::general_purpose::STANDARD.encode(bytes)}))
        }
    }.await;
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
    use std::path::Path;

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
            let c = db.create_conversation("teste").unwrap();
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
        let recalled = core.recall(&prefs, "Qual o nome do meu pet?", &token).await;
        show(&format!("busca pet ({} ms)", start.elapsed().as_millis()), &recalled);
        assert!(recalled.iter().any(|f| f.content.contains("Mel")));
        let unrelated = core.recall(&prefs, "Quanto é 17 vezes 23?", &token).await;
        show("busca sem relação", &unrelated);

        let second = talk(&[("user", "Novidade: me mudei para Recife semana passada."), ("assistant", "Que mudança! Boa adaptação em Recife.")]);
        let updated = core.remember(&prefs, &second, "auto", &token).await.unwrap();
        show("atualização", &updated);
        let all = core.store.lock().unwrap().facts().unwrap();
        show("todas", &all);
        assert!(all.iter().any(|f| f.content.contains("Recife")));
        assert!(!all.iter().any(|f| f.content.contains("Fortaleza")), "a cidade antiga deve ser substituída");

        let third = talk(&[("user", "Pode esquecer a informação sobre a minha gata."), ("assistant", "Tudo bem, esqueci.")]);
        core.remember(&prefs, &third, "explicit", &token).await.unwrap();
        let all = core.store.lock().unwrap().facts().unwrap();
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
