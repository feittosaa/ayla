use super::{
    hardware,
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
    time::Instant,
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
        self.runtime.stop();
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
        json!({"system_prompt":prefs.system_prompt(),"preferences":prefs,"memory_budget_bytes":budget,"hardware":hw,"models":installed,"runtimes":artifacts,"worker":state.runtime.status(),"metrics":metrics,"recommended_profile":recommended,"catalog":catalog,"recommended_model":recommended_model}),
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
    state.runtime.stop();
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
    let result = async {
        let runtimes = state.root.join("runtimes");
        let llama = runtime::artifacts()
            .into_iter()
            .find(|a| a.id == runtime::preferred_llama())
            .ok_or("Componente desconhecido")?;
        let runtime_bytes = if runtime::llama_installed(&runtimes) { 0 } else { llama.bytes };
        let total = runtime_bytes + entry.download_bytes();
        if runtime_bytes > 0 {
            runtime::install(&runtimes, llama.id, &token, |event| {
                let received = event["received"].as_u64().unwrap_or(0);
                let _ = events.send(json!({"type":"download","received":received,"total":total}));
            })
            .await?;
        }
        let models_root = state.root.join("models");
        fs::create_dir_all(&models_root).map_err(|_| "Não foi possível criar a pasta de modelos")?;
        // Restos de downloads interrompidos por encerramento abrupto; nunca registrados.
        if let Ok(entries) = fs::read_dir(&models_root) {
            for item in entries.flatten() {
                if item.file_name().to_string_lossy().ends_with(".installing") {
                    let _ = fs::remove_dir_all(item.path());
                }
            }
        }
        let model = runtime::install_model(&models_root, entry, &token, |received| {
            let _ = events.send(json!({"type":"download","received":runtime_bytes + received,"total":total}));
        })
        .await?;
        let definition = serde_json::to_string(&model).map_err(|_| "Modelo inválido")?;
        let saved = state
            .store
            .lock()
            .map_err(|_| "Dados ocupados".to_string())
            .and_then(|db| db.save_model(&model.id, &definition).map_err(|_| "Não foi possível registrar modelo".to_string()));
        if let Err(error) = saved {
            if let Some(directory) = model.path.parent() {
                let _ = fs::remove_dir_all(directory);
            }
            return Err(error);
        }
        Ok(())
    }
    .await;
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
    state.runtime.stop();
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
    let _gate = state
        .runtime
        .gate
        .try_lock()
        .map_err(|_| "Já existe uma tarefa de IA em andamento")?;
    let token = state.job(&request.id)?;
    let start = Instant::now();
    let mut partial = String::new();
    let mut first = None;
    let mut chunks = 0;
    let mut metric_model = String::from("unresolved");
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
        context.insert(0,json!({"role":"system","content":prefs.system_prompt()}));
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
            let _=events.send(json!({"type":"routing","text":format!("{} · {:?}",model.name,kind)}));
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
        let (model_id, vector) = if use_cloud {
            cloud_embedding(&state, &prefs, &request, &token).await?
        } else {
            let model = models::choose(
                &state.models()?,
                Capability::Embedding,
                state.budget(&prefs),
                prefs.context_tokens,
                None,
            )?;
            let vector = state
                .runtime
                .embed(&model, &prefs, &request.text, &token)
                .await?;
            (model.id, vector)
        };
        let db = state.store.lock().map_err(|_| "Dados ocupados")?;
        if !search {
            db.save_memory(&request.text, &model_id, &vector)
                .map_err(|_| "Não foi possível salvar memória")?;
            return Ok(json!({"saved":true}));
        }
        let mut results: Vec<_> = db
            .memories(&model_id)
            .map_err(|_| "Falha ao consultar memórias")?
            .into_iter()
            .filter_map(|(content, stored)| {
                cosine(&vector, &stored).map(|score| json!({"content":content,"score":score}))
            })
            .collect();
        results.sort_by(|a, b| {
            b["score"]
                .as_f64()
                .unwrap_or(0.0)
                .total_cmp(&a["score"].as_f64().unwrap_or(0.0))
        });
        results.truncate(5);
        Ok(json!({"results":results}))
    }
    .await;
    state.finish(&request.id);
    result
}
fn cosine(a: &[f32], b: &[f32]) -> Option<f64> {
    if a.is_empty() || a.len() != b.len() {
        return None;
    }
    let (dot, aa, bb) = a
        .iter()
        .zip(b)
        .fold((0.0f64, 0.0, 0.0), |(dot, aa, bb), (&a, &b)| {
            (
                dot + f64::from(a) * f64::from(b),
                aa + f64::from(a).powi(2),
                bb + f64::from(b).powi(2),
            )
        });
    if aa == 0.0 || bb == 0.0 || !dot.is_finite() {
        None
    } else {
        Some(dot / (aa.sqrt() * bb.sqrt()))
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn semantic_similarity_rejects_wrong_dimensions_and_empty_vectors() {
        assert_eq!(cosine(&[1.0, 0.0], &[1.0, 0.0]), Some(1.0));
        assert!(cosine(&[1.0], &[1.0, 2.0]).is_none());
        assert!(cosine(&[0.0], &[0.0]).is_none());
    }
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
