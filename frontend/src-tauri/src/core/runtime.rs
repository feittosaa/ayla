use super::{
    hardware,
    models::{Capability, CatalogEntry, Model},
    preferences::Preferences,
};
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read},
    net::TcpListener,
    path::{Component, Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Serialize)]
pub struct Artifact {
    pub id: &'static str,
    pub label: &'static str,
    pub bytes: u64,
    pub url: &'static str,
    pub sha256: &'static str,
    pub binary: &'static str,
}
pub fn artifacts() -> Vec<Artifact> {
    vec![
 Artifact{id:"llama-vulkan",label:"Texto, raciocínio, visão e memória · GPU Linux",bytes:36968023,url:"https://github.com/ggml-org/llama.cpp/releases/download/b12531/llama-b12531-bin-ubuntu-vulkan-x64.tar.gz",sha256:"38eaab84338b9c7ec6be450856e0add797526d20e37c28b5c4430cadde8f1a9b",binary:"llama-server"},
 Artifact{id:"llama-cpu",label:"Texto, raciocínio, visão e memória · CPU",bytes:23901164,url:"https://github.com/ggml-org/llama.cpp/releases/download/b12531/llama-b12531-bin-ubuntu-x64.tar.gz",sha256:"5c1bf73c8c338d7934bbe7b43827496685f69b2e2afce2de9c536f755dd94a20",binary:"llama-server"},
 Artifact{id:"whisper",label:"Transcrição de voz",bytes:10364195,url:"https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-ubuntu-x64.tar.gz",sha256:"a72becf15d7917f990f6313867a52638b82b7f9ef237fb0c980dac56a135781c",binary:"whisper-cli"},
 Artifact{id:"speech",label:"Síntese de voz",bytes:28990228,url:"https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.12.32/sherpa-onnx-v1.12.32-linux-x64-shared.tar.bz2",sha256:"347738b19874237a8f29be6f33f4b55bd1ee6d62f604a59e22fd2f91385bbfb0",binary:"sherpa-onnx-offline-tts"},
]
}

pub fn find_binary(root: &Path, name: &str) -> Option<PathBuf> {
    if !root.is_dir() {
        return None;
    }
    for entry in fs::read_dir(root).ok()?.flatten() {
        let kind = entry.file_type().ok()?;
        if (kind.is_file() || kind.is_symlink())
            && entry.file_name() == name
            && entry.path().is_file()
        {
            return Some(entry.path());
        }
        if kind.is_dir() {
            if let Some(path) = find_binary(&entry.path(), name) {
                return Some(path);
            }
        }
    }
    None
}

fn extract(
    archive: &Path,
    destination: &Path,
    bzip: bool,
    cancel: &CancellationToken,
) -> io::Result<()> {
    let file = fs::File::open(archive)?;
    let reader: Box<dyn Read> = if bzip {
        Box::new(bzip2::read::BzDecoder::new(file))
    } else {
        Box::new(flate2::read::GzDecoder::new(file))
    };
    let mut archive = tar::Archive::new(reader);
    let mut expanded = 0u64;
    for entry in archive.entries()? {
        if cancel.is_cancelled() {
            return Err(io::Error::other("Instalação interrompida"));
        }
        let mut entry = entry?;
        if entry.path()?.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err(io::Error::other("Caminho de pacote inválido"));
        }
        if let Some(link) = entry.link_name()? {
            if link.components().any(|c| {
                matches!(
                    c,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            }) {
                return Err(io::Error::other("Atalho inválido no pacote"));
            }
        }
        expanded = expanded.saturating_add(entry.size());
        if expanded > 2 * 1024 * 1024 * 1024 {
            return Err(io::Error::other("Pacote excede o tamanho permitido"));
        }
        if !entry.unpack_in(destination)? {
            return Err(io::Error::other("Entrada inválida no pacote"));
        }
    }
    Ok(())
}

/// Cache local opcional de downloads (desenvolvimento e testes): `AYLA_MODEL_CACHE`.
/// Arquivos são achados pelo nome da URL e só usados se tamanho e SHA-256 conferirem.
fn download_cache(url: &str) -> Option<PathBuf> {
    let directory = PathBuf::from(std::env::var_os("AYLA_MODEL_CACHE")?);
    let name = url.split('?').next()?.rsplit('/').next()?;
    (directory.is_dir() && !name.is_empty() && !name.contains("..")).then(|| directory.join(name))
}
/// Reaproveita o arquivo do cache se tamanho e SHA-256 conferirem, com progresso e interrupção.
async fn from_cache(
    cached: &Path,
    bytes: u64,
    sha256: &str,
    destination: &Path,
    cancel: &CancellationToken,
    progress: &mut impl FnMut(u64),
) -> bool {
    use tokio::io::AsyncReadExt;
    if fs::metadata(cached).map(|m| m.len()).ok() != Some(bytes) {
        return false;
    }
    let Ok(mut file) = tokio::fs::File::open(cached).await else { return false };
    let mut sha = Sha256::new();
    let mut buffer = vec![0u8; 4 << 20];
    let mut read = 0u64;
    loop {
        if cancel.is_cancelled() {
            return false;
        }
        let Ok(n) = file.read(&mut buffer).await else { return false };
        if n == 0 {
            break;
        }
        sha.update(&buffer[..n]);
        read += n as u64;
        progress(read);
    }
    format!("{:x}", sha.finalize()) == sha256
        && (fs::hard_link(cached, destination).is_ok() || fs::copy(cached, destination).is_ok())
}

/// Baixa `url` para `destination`, exigindo tamanho e SHA-256 exatos.
/// Em erro ou cancelamento o arquivo parcial é removido.
pub async fn download(
    url: &str,
    bytes: u64,
    sha256: &str,
    destination: &Path,
    cancel: &CancellationToken,
    progress: impl FnMut(u64),
) -> Result<(), String> {
    fetch(url, bytes, sha256, destination, cancel, progress, download_cache(url)).await
}
async fn fetch(
    url: &str,
    bytes: u64,
    sha256: &str,
    destination: &Path,
    cancel: &CancellationToken,
    mut progress: impl FnMut(u64),
    cached: Option<PathBuf>,
) -> Result<(), String> {
    if let Some(cached) = &cached {
        if from_cache(cached, bytes, sha256, destination, cancel, &mut progress).await {
            return Ok(());
        }
        if cancel.is_cancelled() {
            return Err("Download interrompido".into());
        }
    }
    let result = async {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(60))
            .build()
            .map_err(|_| "Falha ao preparar download")?;
        let response = tokio::select! { _=cancel.cancelled()=>return Err("Download interrompido".to_string()), response=client.get(url).send()=>response.map_err(|_|"Não foi possível iniciar o download. Verifique a conexão.")? };
        let response = response
            .error_for_status()
            .map_err(|_| "O servidor não disponibilizou este arquivo")?;
        let mut file = tokio::fs::File::create(destination)
            .await
            .map_err(|_| "Não foi possível salvar download")?;
        let mut stream = response.bytes_stream();
        let mut sha = Sha256::new();
        let mut received = 0u64;
        loop {
            let item = tokio::select! {_=cancel.cancelled()=>return Err("Download interrompido".to_string()),item=stream.next()=>item};
            let Some(item) = item else { break };
            let chunk = item.map_err(|_| "Download interrompido pela conexão")?;
            received += chunk.len() as u64;
            if received > bytes {
                return Err("Arquivo com tamanho inesperado".into());
            }
            sha.update(&chunk);
            file.write_all(&chunk)
                .await
                .map_err(|_| "Sem espaço em disco para concluir o download")?;
            progress(received);
        }
        file.sync_all()
            .await
            .map_err(|_| "Não foi possível concluir o download")?;
        if received != bytes || format!("{:x}", sha.finalize()) != sha256 {
            return Err("O arquivo não passou na verificação de integridade".into());
        }
        Ok(())
    }
    .await;
    match (&result, &cached) {
        (Err(_), _) => {
            let _ = fs::remove_file(destination);
        }
        (Ok(()), Some(cached)) if !cached.exists() => {
            let _ = fs::hard_link(destination, cached).or_else(|_| fs::copy(destination, cached).map(|_| ()));
        }
        _ => {}
    }
    result
}

/// Pacote llama.cpp adequado: Vulkan apenas com GPU dedicada e carregador Vulkan presente.
pub fn preferred_llama() -> &'static str {
    let gpu = hardware::detect()
        .gpus
        .iter()
        .any(|g| g.vram_total_bytes.unwrap_or(0) >= 2 << 30);
    let loader = [
        "/usr/lib64/libvulkan.so.1",
        "/usr/lib/x86_64-linux-gnu/libvulkan.so.1",
        "/usr/lib/libvulkan.so.1",
    ]
    .iter()
    .any(|path| Path::new(path).is_file());
    if gpu && loader {
        "llama-vulkan"
    } else {
        "llama-cpu"
    }
}
pub fn llama_installed(root: &Path) -> bool {
    ["llama-vulkan", "llama-cpu"]
        .iter()
        .any(|id| find_binary(&root.join(id), "llama-server").is_some())
}

pub async fn install(
    root: &Path,
    id: &str,
    cancel: &CancellationToken,
    mut report: impl FnMut(Value),
) -> Result<(), String> {
    if std::env::consts::ARCH != "x86_64" {
        return Err("Os pacotes gerenciados desta versão são para Linux x86_64".into());
    }
    let artifact = artifacts()
        .into_iter()
        .find(|a| a.id == id)
        .ok_or("Componente desconhecido")?;
    let target = root.join(id);
    if find_binary(&target, artifact.binary).is_some() {
        return Ok(());
    }
    fs::create_dir_all(root).map_err(|_| "Não foi possível criar a pasta de componentes")?;
    let temporary = root.join(format!("{id}.part"));
    let staging = root.join(format!("{id}.installing"));
    let result = async {
        download(artifact.url, artifact.bytes, artifact.sha256, &temporary, cancel, |received| {
            report(json!({"type":"download","received":received,"total":artifact.bytes}))
        })
        .await?;
        let _ = fs::remove_dir_all(&staging);
        fs::create_dir_all(&staging).map_err(|_| "Não foi possível preparar componente")?;
        let source = temporary.clone();
        let dest = staging.clone();
        let cancellation = cancel.clone();
        tokio::task::spawn_blocking(move || extract(&source, &dest, artifact.url.ends_with(".bz2"), &cancellation))
            .await
            .map_err(|_| "Falha ao extrair componente")?
            .map_err(|_| "Pacote inválido ou instalação interrompida")?;
        if find_binary(&staging, artifact.binary).is_none() {
            return Err("O pacote não inclui o componente esperado".into());
        }
        if cancel.is_cancelled() {
            return Err("Instalação interrompida".into());
        }
        fs::rename(&staging, &target).map_err(|_| "Não foi possível concluir a instalação")?;
        Ok(())
    }
    .await;
    let _ = fs::remove_file(&temporary);
    if result.is_err() {
        let _ = fs::remove_dir_all(staging);
    }
    result
}

/// Baixa uma entrada do catálogo para `models/<id>` e devolve o registro pronto.
/// Nada fica registrado ou visível em `models` antes de todos os arquivos passarem na verificação.
pub async fn install_model(
    models_root: &Path,
    entry: &CatalogEntry,
    cancel: &CancellationToken,
    mut progress: impl FnMut(u64),
) -> Result<Model, String> {
    let id = super::store::id();
    let staging = models_root.join(format!("{id}.installing"));
    let target = models_root.join(&id);
    fs::create_dir_all(&staging).map_err(|_| "Não foi possível preparar a pasta de modelos")?;
    let result = async {
        let transcription = entry.capabilities.contains(&Capability::Transcription);
        let weights_name = if transcription { "model.bin" } else { "model.gguf" };
        let mut done = 0u64;
        for file in entry.files {
            let name = if file.projector { "projector.gguf" } else if file.archive { "package.tar.bz2" } else { weights_name };
            let path = staging.join(name);
            download(file.url, file.bytes, file.sha256, &path, cancel, |received| {
                progress(done + received)
            })
            .await?;
            if file.archive {
                let (source, dest, cancellation) = (path.clone(), staging.clone(), cancel.clone());
                tokio::task::spawn_blocking(move || extract(&source, &dest, true, &cancellation))
                    .await
                    .map_err(|_| "Falha ao extrair modelo")?
                    .map_err(|_| "Pacote de modelo inválido ou instalação interrompida")?;
                let _ = fs::remove_file(&path);
            }
            done += file.bytes;
        }
        if cancel.is_cancelled() {
            return Err("Download interrompido".to_string());
        }
        let path = if entry.files.iter().any(|f| f.archive) {
            let found = find_binary(&staging, "model.onnx").ok_or("O pacote não contém o modelo de voz esperado")?;
            target.join(found.strip_prefix(&staging).map_err(|_| "Pacote de modelo inválido")?)
        } else {
            target.join(weights_name)
        };
        fs::rename(&staging, &target).map_err(|_| "Não foi possível concluir a instalação do modelo")?;
        let weights = entry
            .files
            .iter()
            .find(|f| !f.projector)
            .ok_or("Catálogo sem arquivo de pesos")?;
        Ok(Model {
            id: id.clone(),
            name: entry.name.into(),
            capability: entry.capabilities[0],
            capabilities: entry.capabilities.to_vec(),
            path,
            companion: entry
                .files
                .iter()
                .any(|f| f.projector)
                .then(|| target.join("projector.gguf")),
            bytes: entry.download_bytes(),
            sha256: weights.sha256.into(),
            catalog: Some(entry.id.into()),
            memory_bytes: Some(entry.memory_bytes()),
            speaker: entry.speaker,
        })
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

struct Worker {
    child: Child,
    model_id: String,
    url: String,
    token: String,
    last_used: Instant,
    idle_seconds: u64,
    context: u32,
}
pub struct Runtime {
    pub root: PathBuf,
    worker: Mutex<Option<Worker>>,
    /// Embeddings da memória: processo leve na CPU, ao lado do modelo de conversa.
    memory: Mutex<Option<Worker>>,
    pub gate: tokio::sync::Mutex<()>,
    client: reqwest::Client,
}
impl Runtime {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            worker: Mutex::new(None),
            memory: Mutex::new(None),
            gate: tokio::sync::Mutex::new(()),
            client: reqwest::Client::builder()
                .no_proxy()
                .connect_timeout(Duration::from_secs(3))
                .read_timeout(Duration::from_secs(120))
                .build()
                .unwrap(),
        }
    }
    fn stop_slot(slot: &Mutex<Option<Worker>>) {
        if let Ok(mut worker) = slot.lock() {
            if let Some(mut worker) = worker.take() {
                let _ = worker.child.kill();
                let _ = worker.child.wait();
            }
        }
    }
    /// Encerra o modelo de conversa/voz; o worker de memória segue seu próprio ciclo.
    pub fn stop(&self) {
        Self::stop_slot(&self.worker);
    }
    pub fn stop_all(&self) {
        Self::stop_slot(&self.worker);
        Self::stop_slot(&self.memory);
    }
    fn slot(&self, model: &Model) -> &Mutex<Option<Worker>> {
        if model.capability == Capability::Embedding {
            &self.memory
        } else {
            &self.worker
        }
    }
    pub fn idle_check(&self) {
        let idle = |slot: &Mutex<Option<Worker>>| {
            slot.lock()
                .ok()
                .and_then(|w| {
                    w.as_ref()
                        .map(|w| w.last_used.elapsed() > Duration::from_secs(w.idle_seconds))
                })
                .unwrap_or(false)
        };
        for slot in [&self.worker, &self.memory] {
            if idle(slot) {
                if let Ok(_guard) = self.gate.try_lock() {
                    Self::stop_slot(slot);
                }
            }
        }
    }
    fn slot_bytes(slot: &Mutex<Option<Worker>>) -> u64 {
        slot.lock()
            .ok()
            .and_then(|w| w.as_ref().map(|w| w.child.id()))
            .and_then(|pid| fs::read_to_string(format!("/proc/{pid}/status")).ok())
            .and_then(|text| {
                text.lines()
                    .find_map(|line| {
                        line.strip_prefix("VmRSS:")
                            .and_then(|value| value.split_whitespace().next()?.parse::<u64>().ok())
                    })
                    .map(|kb| kb * 1024)
            })
            .unwrap_or(0)
    }
    pub fn resident_bytes(&self) -> u64 {
        Self::slot_bytes(&self.worker) + Self::slot_bytes(&self.memory)
    }
    pub fn status(&self) -> Value {
        let id = |slot: &Mutex<Option<Worker>>| {
            slot.lock().ok().and_then(|w| w.as_ref().map(|w| w.model_id.clone()))
        };
        json!({"active_model":id(&self.worker),"memory_model":id(&self.memory),"resident_bytes":self.resident_bytes()})
    }
    fn command(&self, component: &str, binary: &str) -> Result<Command, String> {
        let root = self.root.join(component);
        let executable = find_binary(&root, binary)
            .ok_or("Instale o componente necessário em Modelos antes de usar esta capacidade")?;
        let mut command = Command::new(&executable);
        command.env(
            "LD_LIBRARY_PATH",
            format!(
                "{}:{}:{}",
                executable.parent().unwrap().display(),
                root.join("lib").display(),
                executable.parent().unwrap().join("../lib").display()
            ),
        );
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        Ok(command)
    }
    async fn ensure(
        &self,
        model: &Model,
        prefs: &Preferences,
        cancel: &CancellationToken,
    ) -> Result<(String, String), String> {
        let memory = model.capability == Capability::Embedding;
        let slot = self.slot(model);
        let stop = || Self::stop_slot(slot);
        // Embeddings usam contexto fixo: entradas curtas e um processo pequeno.
        let context = if memory { 2048 } else { prefs.context_tokens };
        {
            let mut guard = slot.lock().map_err(|_| "Modelo ocupado")?;
            if let Some(worker) = guard.as_mut() {
                if worker.model_id == model.id
                    && worker.context == context
                    && worker
                        .child
                        .try_wait()
                        .map_err(|_| "Falha no modelo")?
                        .is_none()
                {
                    worker.last_used = Instant::now();
                    worker.idle_seconds = prefs.idle_seconds;
                    return Ok((worker.url.clone(), worker.token.clone()));
                }
            }
        }
        stop();
        let has = |id: &str| find_binary(&self.root.join(id), "llama-server").is_some();
        let component = if memory {
            if has("llama-cpu") { "llama-cpu" } else { "llama-vulkan" }
        } else if has("llama-vulkan") {
            "llama-vulkan"
        } else {
            "llama-cpu"
        };
        let mut command = self.command(component, "llama-server")?;
        let port = TcpListener::bind("127.0.0.1:0")
            .map_err(|_| "Não foi possível iniciar modelo")?
            .local_addr()
            .map_err(|_| "Falha na porta local")?
            .port();
        let mut random = [0u8; 32];
        fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut random))
            .map_err(|_| "Falha ao proteger modelo")?;
        let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
        command.arg("-m").arg(&model.path).args([
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "-c",
            &context.to_string(),
            "--parallel",
            "1",
            "--threads",
            &{
                let cpus = hardware::detect().logical_cpus;
                if memory { (cpus / 4).max(1) } else { cpus.saturating_sub(1).max(1) }
            }
            .to_string(),
            "--no-warmup",
            "--offline",
            "--no-webui",
            "--no-slots",
        ]);
        command.env("LLAMA_API_KEY", &token);
        if component == "llama-cpu" || memory {
            command.args(["-ngl", "0"]);
        } else {
            let hw = hardware::detect();
            let vram = hw
                .gpus
                .iter()
                .filter_map(|g| g.vram_total_bytes)
                .max()
                .unwrap_or(0);
            let margin = ((vram.saturating_mul(100 - u64::from(prefs.percent())) / 100)
                / (1024 * 1024))
                .max(512);
            command.args([
                "-ngl",
                "auto",
                "--split-mode",
                "none",
                "--fit-target",
                &margin.to_string(),
            ]);
        }
        if let Some(projector) = &model.companion {
            command.arg("--mmproj").arg(projector);
        }
        if model.capability == Capability::Embedding {
            // O GGUF informa pooling; não impor mean a modelos que exigem CLS/last.
            command.args(["--embedding", "-ub", "2048", "-b", "2048"]);
        }
        let child=command.spawn().map_err(|_|"Não foi possível executar o modelo. Verifique a compatibilidade do componente com seu Linux.")?;
        let url = format!("http://127.0.0.1:{port}");
        *slot.lock().map_err(|_| "Modelo ocupado")? = Some(Worker {
            child,
            model_id: model.id.clone(),
            url: url.clone(),
            token: token.clone(),
            last_used: Instant::now(),
            idle_seconds: prefs.idle_seconds,
            context,
        });
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(120) {
            if cancel.is_cancelled() {
                stop();
                return Err("Carregamento interrompido".into());
            }
            let dead = slot
                .lock()
                .map_err(|_| "Modelo ocupado")?
                .as_mut()
                .and_then(|w| w.child.try_wait().ok().flatten())
                .is_some();
            if dead {
                stop();
                return Err(
                    "O modelo não carregou. Verifique formato, projetor e orçamento de memória."
                        .into(),
                );
            }
            if let Ok(response) = self
                .client
                .get(format!("{url}/health"))
                .bearer_auth(&token)
                .send()
                .await
            {
                if response.status().is_success() {
                    return Ok((url, token));
                }
            }
            tokio::select! {_=cancel.cancelled()=>{stop();return Err("Carregamento interrompido".into());},_=tokio::time::sleep(Duration::from_millis(100))=>{}}
        }
        stop();
        Err("Tempo de carregamento excedido".into())
    }
    pub async fn local_chat(
        &self,
        model: &Model,
        kind: Capability,
        prefs: &Preferences,
        messages: Vec<Value>,
        cancel: &CancellationToken,
        report: impl FnMut(Value),
    ) -> Result<(String, Option<u64>, u64), String> {
        let (url, token) = self.ensure(model, prefs, cancel).await?;
        let request = self
            .client
            .post(format!("{url}/v1/chat/completions"))
            .bearer_auth(token)
            .json(&chat_body(kind, prefs, messages));
        let result = stream_chat(request, cancel, report).await;
        if cancel.is_cancelled() {
            self.stop();
        } else {
            if let Ok(mut w) = self.worker.lock() {
                if let Some(w) = w.as_mut() {
                    w.last_used = Instant::now();
                }
            }
        }
        result
    }
    pub async fn embed(
        &self,
        model: &Model,
        prefs: &Preferences,
        text: &str,
        cancel: &CancellationToken,
    ) -> Result<Vec<f32>, String> {
        self.embed_many(model, prefs, &[text.to_string()], cancel)
            .await?
            .pop()
            .ok_or_else(|| "Vetor inválido".into())
    }
    pub async fn embed_many(
        &self,
        model: &Model,
        prefs: &Preferences,
        texts: &[String],
        cancel: &CancellationToken,
    ) -> Result<Vec<Vec<f32>>, String> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let (url, token) = self.ensure(model, prefs, cancel).await?;
        let response = tokio::select! {_=cancel.cancelled()=>{Self::stop_slot(&self.memory);return Err("Tarefa interrompida".into());},r=self.client.post(format!("{url}/v1/embeddings")).bearer_auth(token).json(&json!({"input":texts})).send()=>r.map_err(|_|"Falha no modelo de memória")?};
        let data: Value = response
            .error_for_status()
            .map_err(|_| "Modelo incompatível com embeddings")?
            .json()
            .await
            .map_err(|_| "Resposta de memória inválida")?;
        let mut items: Vec<(usize, Vec<f32>)> = Vec::new();
        for (position, item) in data["data"].as_array().ok_or("Resposta de memória inválida")?.iter().enumerate() {
            let vector: Vec<f32> = serde_json::from_value(item["embedding"].clone()).map_err(|_| "Vetor inválido")?;
            if vector.is_empty() || vector.iter().any(|n| !n.is_finite()) {
                return Err("Vetor inválido".into());
            }
            items.push((item["index"].as_u64().map(|i| i as usize).unwrap_or(position), vector));
        }
        items.sort_by_key(|(i, _)| *i);
        if items.len() != texts.len() {
            return Err("Resposta de memória incompleta".into());
        }
        if let Ok(mut w) = self.memory.lock() {
            if let Some(w) = w.as_mut() {
                w.last_used = Instant::now();
            }
        }
        Ok(items.into_iter().map(|(_, v)| v).collect())
    }
    /// Geração não interativa com saída restrita a um JSON Schema (extração de memórias).
    /// Cancelar abandona o pedido sem derrubar o modelo, que segue carregado para a conversa.
    pub async fn complete_json(
        &self,
        model: &Model,
        prefs: &Preferences,
        messages: Vec<Value>,
        schema: Value,
        cancel: &CancellationToken,
    ) -> Result<String, String> {
        let (url, token) = self.ensure(model, prefs, cancel).await?;
        let body = json!({"messages":messages,"max_tokens":700,"temperature":0.2,
            "chat_template_kwargs":{"enable_thinking":false},
            "response_format":{"type":"json_schema","json_schema":{"name":"memorias","schema":schema}}});
        let response = tokio::select! {_=cancel.cancelled()=>return Err("Tarefa interrompida".into()),r=self.client.post(format!("{url}/v1/chat/completions")).bearer_auth(token).json(&body).send()=>r.map_err(|_|"Falha ao consultar o modelo")?};
        let data: Value = tokio::select! {_=cancel.cancelled()=>return Err("Tarefa interrompida".into()),r=async{response.error_for_status().map_err(|_|"O modelo recusou o pedido")?.json::<Value>().await.map_err(|_|"Resposta inválida")}=>r?};
        if let Ok(mut w) = self.worker.lock() {
            if let Some(w) = w.as_mut() {
                w.last_used = Instant::now();
            }
        }
        if data["choices"][0]["finish_reason"] == "length" {
            return Err("Extração excedeu o limite".into());
        }
        data["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| "Resposta sem conteúdo".into())
    }
    /// Linha de comando do sherpa-onnx para o modelo de voz: Kokoro (pasta com
    /// voices.bin) ou VITS/Piper. Sem Python; o texto já deve vir normalizado.
    fn speech_command(&self, model: &Model, text: &str, output: &Path) -> Result<Command, String> {
        let dir = model.path.parent().ok_or("Modelo de voz inválido")?;
        let mut c = self.command("speech", "sherpa-onnx-offline-tts")?;
        let arg = |name: &str, path: &Path| format!("--{name}={}", path.display());
        let data = dir.join("espeak-ng-data");
        if dir.join("voices.bin").is_file() {
            c.arg(arg("kokoro-model", &model.path))
                .arg(arg("kokoro-voices", &dir.join("voices.bin")))
                .arg(arg("kokoro-tokens", &dir.join("tokens.txt")))
                .arg(arg("kokoro-data-dir", &data))
                .arg("--kokoro-lang=pt-br")
                .arg(format!("--sid={}", model.speaker.unwrap_or(0)));
        } else {
            c.arg(arg("vits-model", &model.path)).arg(arg("vits-tokens", &dir.join("tokens.txt")));
            if data.is_dir() {
                c.arg(arg("vits-data-dir", &data));
            }
            if dir.join("lexicon.txt").is_file() {
                c.arg(arg("vits-lexicon", &dir.join("lexicon.txt")));
            }
        }
        let threads = (hardware::detect().logical_cpus / 2).max(1);
        // O texto vai como último argumento; nunca começa com "-" (prepare garante).
        c.arg(format!("--num-threads={threads}"))
            .arg(arg("output-filename", output))
            .arg(text.trim_start_matches('-'));
        Ok(c)
    }
    /// Transcrição (whisper.cpp) ou síntese (sherpa-onnx) em processo próprio,
    /// sem descarregar o modelo de conversa. Cancelar encerra só este processo.
    pub async fn voice(
        &self,
        model: &Model,
        input: &Path,
        text: &str,
        output: &Path,
        cancel: &CancellationToken,
    ) -> Result<(), String> {
        let mut command = if model.capability == Capability::Transcription {
            let mut c = self.command("whisper", "whisper-cli")?;
            let threads = hardware::detect().logical_cpus.saturating_sub(1).max(1);
            c.args(["-m"])
                .arg(&model.path)
                .args(["-f"])
                .arg(input)
                .args(["-l", "auto", "-nt", "-np", "-t", &threads.to_string(), "-otxt", "-of"])
                .arg(output);
            c
        } else {
            if text.trim().is_empty() {
                return Err("Não há texto para falar".into());
            }
            self.speech_command(model, text, output)?
        };
        let mut child = command
            .spawn()
            .map_err(|_| "Não foi possível iniciar componente de voz")?;
        let start = Instant::now();
        loop {
            if cancel.is_cancelled() || start.elapsed() > Duration::from_secs(180) {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Tarefa de voz interrompida".into());
            }
            if let Some(status) = child.try_wait().map_err(|_| "Falha no componente de voz")? {
                return if status.success() {
                    Ok(())
                } else {
                    Err("O componente de voz não conseguiu usar este modelo ou arquivo".into())
                };
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.stop_all();
    }
}

/// Corpo do pedido local. Modelos híbridos (ex.: Qwen3.5) raciocinam por
/// padrão; o modo só fica ligado quando o pedido foi roteado para raciocínio.
/// Templates sem essa opção a ignoram.
fn chat_body(kind: Capability, prefs: &Preferences, messages: Vec<Value>) -> Value {
    json!({
        "messages": messages,
        "stream": true,
        "max_tokens": prefs.context_tokens / 2,
        "chat_template_kwargs": {"enable_thinking": kind == Capability::Reasoning},
    })
}

pub async fn stream_chat(
    request: reqwest::RequestBuilder,
    cancel: &CancellationToken,
    mut report: impl FnMut(Value),
) -> Result<(String, Option<u64>, u64), String> {
    if cancel.is_cancelled() {
        return Err("Resposta interrompida".into());
    }
    let start = Instant::now();
    let mut first = None;
    let mut tokens = 0;
    let mut full = String::new();
    let response = tokio::select! {_=cancel.cancelled()=>return Err("Resposta interrompida".into()),r=request.send()=>r.map_err(|_|"Não foi possível conectar ao modelo")?};
    let mut stream = response
        .error_for_status()
        .map_err(|_| "O modelo recusou o pedido")?
        .bytes_stream();
    let mut buffer = Vec::new();
    let mut done = false;
    loop {
        let item = tokio::select! {_=cancel.cancelled()=>return Err("Resposta interrompida".into()),item=stream.next()=>item};
        let Some(item) = item else {
            break;
        };
        buffer.extend(item.map_err(|_| "Resposta interrompida")?);
        if buffer.len() > 8 * 1024 * 1024 {
            return Err("Resposta excede o limite de processamento".into());
        }
        while let Some(index) = buffer.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = buffer.drain(..=index).collect();
            let line = std::str::from_utf8(&line)
                .map_err(|_| "Resposta inválida")?
                .trim();
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data == "[DONE]" {
                done = true;
                break;
            }
            let event: Value = serde_json::from_str(data).map_err(|_| "Resposta inválida")?;
            if event["choices"][0]["finish_reason"].as_str() == Some("length") {
                return Err("O modelo atingiu o limite de geração. Ajuste o contexto ou use um modelo menor.".into());
            }
            if let Some(token) = event["choices"][0]["delta"]["content"].as_str() {
                if first.is_none() {
                    first = Some(start.elapsed().as_millis() as u64);
                }
                tokens += 1;
                full.push_str(token);
                report(json!({"type":"token","text":token}));
                if full.len() > 4 * 1024 * 1024 {
                    return Err("Resposta excede o limite configurado".into());
                }
            }
        }
        if done {
            break;
        }
    }
    if !done {
        return Err("A conexão terminou sem concluir a resposta".into());
    }
    if full.trim().is_empty() {
        return Err("O modelo não produziu uma resposta final".into());
    }
    Ok((full, first, tokens))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn thinking_is_enabled_only_for_reasoning_requests() {
        let prefs = Preferences::initial();
        for (kind, expected) in [
            (Capability::Text, false),
            (Capability::Vision, false),
            (Capability::Reasoning, true),
        ] {
            let body = chat_body(kind, &prefs, vec![json!({"role":"user","content":"oi"})]);
            assert_eq!(body["chat_template_kwargs"]["enable_thinking"], expected);
            assert_eq!(body["max_tokens"], prefs.context_tokens / 2);
            assert_eq!(body["stream"], true);
        }
    }
    /// Servidor local que responde cada conexão com o corpo indicado pelo caminho.
    fn serve_files(files: Vec<(&'static str, &'static [u8])>, connections: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for _ in 0..connections {
                let Ok((mut socket, _)) = listener.accept() else { return };
                let mut buf = [0u8; 4096];
                let n = socket.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let body = files.iter().find(|(p, _)| *p == path).map(|(_, b)| *b).unwrap_or(b"");
                let _ = write!(socket, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = socket.write_all(body);
            }
        });
        format!("http://{address}")
    }
    fn digest(bytes: &[u8]) -> &'static str {
        Box::leak(format!("{:x}", Sha256::digest(bytes)).into_boxed_str())
    }
    fn entry(base: &str, projector_sha: &'static str) -> CatalogEntry {
        let url = |path: &str| -> &'static str { Box::leak(format!("{base}{path}").into_boxed_str()) };
        CatalogEntry {
            id: "teste",
            name: "Teste",
            summary: "",
            capabilities: &[Capability::Text, Capability::Reasoning, Capability::Vision],
            measured_bytes: 1000,
            runtime: "llama",
            speaker: None,
            files: Box::leak(Box::new([
                super::super::models::CatalogFile { url: url("/w"), bytes: 7, sha256: digest(b"weights"), projector: false, archive: false },
                super::super::models::CatalogFile { url: url("/p"), bytes: 4, sha256: projector_sha, projector: true, archive: false },
            ])),
        }
    }
    #[tokio::test]
    async fn local_cache_is_used_only_when_size_and_hash_match() {
        let root = std::env::temp_dir().join(format!("ayla-cache-{}", super::super::store::id()));
        let cache = root.join("cache");
        fs::create_dir_all(&cache).unwrap();
        let token = CancellationToken::new();
        // Cache válido: nenhum acesso à rede (porta 9 recusaria a conexão).
        fs::write(cache.join("w"), b"weights").unwrap();
        fetch("http://127.0.0.1:9/w", 7, digest(b"weights"), &root.join("a"), &token, |_| {}, Some(cache.join("w"))).await.unwrap();
        assert_eq!(fs::read(root.join("a")).unwrap(), b"weights");
        // Cache adulterado é ignorado e o download verificado o substitui no cache.
        fs::write(cache.join("p"), b"evil").unwrap();
        let base = serve_files(vec![("/p", b"proj")], 1);
        assert!(fetch(&format!("{base}/p"), 4, digest(b"proj"), &root.join("b"), &token, |_| {}, Some(cache.join("p"))).await.is_ok());
        assert_eq!(fs::read(root.join("b")).unwrap(), b"proj");
        assert_eq!(fs::read(cache.join("p")).unwrap(), b"evil", "não sobrescreve arquivo existente do cache");
        let base = serve_files(vec![("/n", b"novo")], 1);
        fetch(&format!("{base}/n"), 4, digest(b"novo"), &root.join("c"), &token, |_| {}, Some(cache.join("n"))).await.unwrap();
        assert_eq!(fs::read(cache.join("n")).unwrap(), b"novo", "download novo alimenta o cache");
        fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn catalog_install_verifies_every_file_before_exposing_the_model() {
        let root = std::env::temp_dir().join(format!("ayla-catalog-{}", super::super::store::id()));
        let base = serve_files(vec![("/w", b"weights"), ("/p", b"proj")], 2);
        let mut last = 0;
        let model = install_model(&root, &entry(&base, digest(b"proj")), &CancellationToken::new(), |r| last = r)
            .await
            .unwrap();
        assert_eq!(last, 11);
        assert_eq!(fs::read(&model.path).unwrap(), b"weights");
        assert_eq!(fs::read(model.companion.as_ref().unwrap()).unwrap(), b"proj");
        assert!(model.supports(Capability::Vision) && model.supports(Capability::Reasoning));
        assert_eq!((model.catalog.as_deref(), model.memory_bytes), (Some("teste"), Some(1200)));
        // Hash divergente no segundo arquivo: nada fica em `models` além do modelo anterior.
        let base = serve_files(vec![("/w", b"weights"), ("/p", b"evil")], 2);
        let error = install_model(&root, &entry(&base, digest(b"proj")), &CancellationToken::new(), |_| {})
            .await
            .unwrap_err();
        assert!(error.contains("integridade"));
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(install_model(&root, &entry("http://127.0.0.1:9", digest(b"proj")), &cancelled, |_| {})
            .await
            .is_err());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn stream_preserves_unicode_and_rejects_incomplete_responses() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = socket.read(&mut buf);
            let body="data: {\"choices\":[{\"delta\":{\"content\":\" Olá 🧡\\n\"}}]}\n\ndata: [DONE]\n\n";
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).unwrap();
            for b in body.as_bytes() {
                socket.write_all(&[*b]).unwrap();
            }
        });
        let mut actual = String::new();
        let request = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!("http://{address}"));
        let result = stream_chat(request, &CancellationToken::new(), |event| {
            actual.push_str(event["text"].as_str().unwrap());
        })
        .await
        .unwrap();
        assert_eq!(actual, " Olá 🧡\n");
        assert_eq!(result.0, actual);
        server.join().unwrap();
    }
    #[tokio::test]
    async fn partial_response_is_preserved_on_eof_and_inflight_cancel() {
        for interrupt in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf);
                let body = "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n";
                write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{body}").unwrap();
                if interrupt {
                    std::thread::sleep(Duration::from_millis(200));
                }
            });
            let token = CancellationToken::new();
            let cancel = token.clone();
            let mut partial = String::new();
            let request = reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap()
                .post(format!("http://{address}"));
            let result = stream_chat(request, &token, |event| {
                partial.push_str(event["text"].as_str().unwrap());
                if interrupt {
                    cancel.cancel();
                }
            })
            .await;
            assert!(result.is_err());
            assert_eq!(partial, "partial");
            server.join().unwrap();
        }
    }
    #[tokio::test]
    async fn cancelled_task_does_not_start_network_request() {
        let token = CancellationToken::new();
        token.cancel();
        let request = reqwest::Client::new().post("http://127.0.0.1:1");
        assert!(
            stream_chat(request, &token, |_| panic!("Sem emissão após cancelamento"))
                .await
                .is_err()
        );
    }
    #[test]
    fn archive_rejects_parent_paths_and_untrusted_links() {
        let root =
            std::env::temp_dir().join(format!("ayla-archive-test-{}", super::super::store::id()));
        fs::create_dir_all(&root).unwrap();
        let archive = root.join("bad.tar.gz");
        let file = fs::File::create(&archive).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut tar = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_cksum();
        tar.append_link(&mut header, "escape", "../outside")
            .unwrap();
        tar.into_inner().unwrap().finish().unwrap();
        assert!(extract(
            &archive,
            &root.join("output"),
            false,
            &CancellationToken::new()
        )
        .is_err());
        assert!(!root.join("outside").exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    #[ignore = "modelos reais autorizados em /tmp; não faz downloads nem altera dados do usuário"]
    async fn real_models_cover_optional_capabilities_and_lifecycle() {
        use base64::Engine;
        let root = PathBuf::from("/tmp/ayla-p1-model-validation");
        let runtime = Runtime::new(PathBuf::from("/tmp/ayla-p1-runtime-validation"));
        let fixture = |file: &str, capability| Model {
            id: format!("test-{capability:?}"),
            name: "temporary fixture".into(),
            capability,
            capabilities: Vec::new(),
            path: root.join(file),
            companion: None,
            bytes: fs::metadata(root.join(file)).unwrap().len(),
            sha256: String::new(),
            catalog: None,
            memory_bytes: None,
            speaker: None,
        };
        let mut prefs = Preferences::default();
        prefs.context_tokens = 2048;
        prefs.idle_seconds = 0;
        let token = CancellationToken::new();
        for capability in [Capability::Text, Capability::Reasoning] {
            let model = fixture("Qwen3-0.6B-Q8_0.gguf", capability);
            let prompt = if capability == Capability::Text {
                "Say hello in Portuguese. /no_think"
            } else {
                "If I have two apples and receive three more, how many apples? Be brief. /think"
            };
            let start = Instant::now();
            let result = runtime
                .local_chat(
                    &model,
                    capability,
                    &prefs,
                    vec![json!({"role":"user","content":prompt})],
                    &token,
                    |_| {},
                )
                .await
                .unwrap();
            eprintln!(
                "{capability:?}: first={:?}ms total={}ms chunks={} rss={} response={}",
                result.1,
                start.elapsed().as_millis(),
                result.2,
                runtime.resident_bytes(),
                result.0
            );
            assert!(!result.0.trim().is_empty());
        }
        let mut vision = fixture("SmolVLM-256M-Instruct-Q8_0.gguf", Capability::Vision);
        vision.companion = Some(root.join("mmproj-SmolVLM-256M-Instruct-Q8_0.gguf"));
        let image = base64::engine::general_purpose::STANDARD
            .encode(fs::read(root.join("cats.png")).unwrap());
        let result = runtime.local_chat(&vision, Capability::Vision, &prefs, vec![json!({"role":"user","content":[{"type":"text","text":"Describe this image briefly."},{"type":"image_url","image_url":{"url":format!("data:image/png;base64,{image}")}}]})], &token, |_| {}).await.unwrap();
        eprintln!("Vision: {}", result.0);
        assert!(
            result.0.to_lowercase().contains("cat"),
            "Vision must recognize the cats in the official sample"
        );
        let embedding = fixture("embeddinggemma-300M-qat-Q4_0.gguf", Capability::Embedding);
        let vector = runtime
            .embed(
                &embedding,
                &prefs,
                "task: search result | title: Test | text: A maçã é uma fruta.",
                &token,
            )
            .await
            .unwrap();
        assert_eq!(vector.len(), 768);
        assert!(vector.iter().any(|n| n.abs() > 0.001));
        eprintln!(
            "Embedding: {} dimensions rss={}",
            vector.len(),
            runtime.resident_bytes()
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
        runtime.idle_check();
        assert!(runtime.status()["active_model"].is_null());
        let speech = fixture(
            "vits-piper-pt_BR-faber-medium/pt_BR-faber-medium.onnx",
            Capability::Speech,
        );
        let wave = root.join("speech.wav");
        runtime
            .voice(
                &speech,
                &wave,
                "Olá! Esta é uma validação temporária da Ayla.",
                &wave,
                &token,
            )
            .await
            .unwrap();
        assert!(fs::metadata(&wave).unwrap().len() > 1000);
        super::super::services::normalize_wav(&wave, &root.join("normalized.wav")).unwrap();
        let whisper = fixture("ggml-tiny.bin", Capability::Transcription);
        runtime
            .voice(
                &whisper,
                &root.join("normalized.wav"),
                "",
                &root.join("transcript"),
                &token,
            )
            .await
            .unwrap();
        let transcript = fs::read_to_string(root.join("transcript.txt")).unwrap();
        assert!(!transcript.trim().is_empty());
        eprintln!("STT from native TTS: {transcript}");
        let model = fixture("Qwen3-0.6B-Q8_0.gguf", Capability::Text);
        let cancel = CancellationToken::new();
        let interrupt = cancel.clone();
        let result = runtime
            .local_chat(
                &model,
                Capability::Text,
                &prefs,
                vec![json!({"role":"user","content":"Count from 1 to 1000. /no_think"})],
                &cancel,
                move |_| interrupt.cancel(),
            )
            .await;
        assert!(result.is_err());
        assert!(runtime.status()["active_model"].is_null());
    }
    #[tokio::test]
    #[ignore = "download explícito das distribuições oficiais em pasta temporária"]
    async fn official_runtime_packages_install_and_include_expected_binaries() {
        let root = std::env::temp_dir().join("ayla-p1-runtime-validation");
        for artifact in artifacts() {
            install(&root, artifact.id, &CancellationToken::new(), |_| {})
                .await
                .unwrap();
            assert!(find_binary(&root.join(artifact.id), artifact.binary).is_some());
        }
    }
}

#[cfg(test)]
mod voice_tests {
    use super::*;
    /// Rode com AYLA_MODEL_CACHE=/var/tmp/ayla-benchmark (arquivos já verificados; sem rede).
    #[tokio::test]
    #[ignore = "modelos de voz reais via cache local; motores de /tmp/ayla-p1-runtime-validation"]
    async fn catalog_voice_speaks_normalized_text_and_transcribes_it_back() {
        let root = std::env::temp_dir().join(format!("ayla-voice-real-{}", super::super::store::id()));
        fs::create_dir_all(root.join("runtimes")).unwrap();
        for id in ["whisper", "speech"] {
            std::os::unix::fs::symlink(format!("/tmp/ayla-p1-runtime-validation/{id}"), root.join("runtimes").join(id)).unwrap();
        }
        let entry = |id: &str| super::super::models::catalog().iter().find(|e| e.id == id).unwrap();
        let token = CancellationToken::new();
        let start = Instant::now();
        let voice = install_model(&root.join("models"), entry("kokoro-dora"), &token, |_| {}).await.unwrap();
        let ear = install_model(&root.join("models"), entry("whisper-small"), &token, |_| {}).await.unwrap();
        eprintln!("instalação via cache: {} ms; voz em {}", start.elapsed().as_millis(), voice.path.display());
        assert!(voice.path.ends_with("model.onnx") && voice.speaker == Some(42));
        let runtime = Runtime::new(root.join("runtimes"));
        let text = super::super::speech_text::prepare("A compra de **R$ 87,90** foi aprovada em 03/11. 😊");
        let wav = root.join("fala.wav");
        let start = Instant::now();
        runtime.voice(&voice, Path::new(""), &text, &wav, &token).await.unwrap();
        let spoken_ms = start.elapsed().as_millis();
        let start = Instant::now();
        runtime.voice(&ear, &wav, "", &root.join("ouvido"), &token).await.unwrap();
        let heard = fs::read_to_string(root.join("ouvido.txt")).unwrap().to_lowercase();
        eprintln!("texto: {text}\nfala: {spoken_ms} ms; transcrição: {} ms → {heard}", start.elapsed().as_millis());
        // O Whisper escreve valores em dígitos; o que importa é ter ouvido o valor e a data.
        assert!(heard.contains("87") && heard.contains("novembro"), "a fala deve dizer o valor e a data por extenso");
        assert!(!text.contains("R$") && !text.contains(".."));
        assert!(runtime.status()["active_model"].is_null(), "voz não usa o worker de conversa");
        fs::remove_dir_all(root).unwrap();
    }
}
