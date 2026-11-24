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
 // Build estático do ffmpeg 7.0.2 (johnvansickle) publicado pelo projeto ffmpeg-static; binário único em .gz.
 Artifact{id:"ffmpeg",label:"Leitura de áudio, vídeo e GIF",bytes:29354986,url:"https://github.com/eugeneware/ffmpeg-static/releases/download/b6.1.1/ffmpeg-linux-x64.gz",sha256:"bfe8a8fc511530457b528c48d77b5737527b504a3797a9bc4866aeca69c2dffa",binary:"ffmpeg"},
 // Ampliador de imagens da Iris: Real-ESRGAN-ncnn-vulkan (MIT) com os modelos oficiais (BSD-3-Clause), pacote 20220424.
 Artifact{id:"upscale",label:"Upscale de imagens (Real-ESRGAN) · placa de vídeo",bytes:46931474,url:"https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.5.0/realesrgan-ncnn-vulkan-20220424-ubuntu.zip",sha256:"e5aa6eb131234b87c0c51f82b89390f5e3e642b7b70f2b9bbe95b6a285a40c96",binary:"realesrgan-ncnn-vulkan"},
 Artifact{id:"whisper",label:"Transcrição de voz",bytes:10364195,url:"https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-ubuntu-x64.tar.gz",sha256:"a72becf15d7917f990f6313867a52638b82b7f9ef237fb0c980dac56a135781c",binary:"whisper-cli"},
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
    cancel: &CancellationToken,
) -> io::Result<()> {
    let file = fs::File::open(archive)?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
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

/// Pacote .zip: extrai só os arquivos pedidos (`wanted`), sem subir de pasta, até 512 MB cada.
/// O pacote já teve o SHA-256 conferido no download.
fn extract_zip(archive: &Path, destination: &Path, wanted: impl Fn(&str) -> bool) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let data = fs::read(archive)?;
    let invalid = || io::Error::other("Pacote .zip inválido");
    let u16_at = |at: usize| data.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize).ok_or_else(invalid);
    let u32_at = |at: usize| data.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize).ok_or_else(invalid);
    // Fim do diretório central: assinatura 0x06054b50 nos últimos 64 KiB.
    let floor = data.len().saturating_sub(22 + 65535);
    let end = (floor..data.len().saturating_sub(21)).rev().find(|&i| data[i..i + 4] == [0x50, 0x4b, 0x05, 0x06]).ok_or_else(invalid)?;
    let (count, mut at) = (u16_at(end + 10)?, u32_at(end + 16)?);
    for _ in 0..count {
        if u32_at(at)? != 0x0201_4b50 { return Err(invalid()); }
        let (method, packed, size) = (u16_at(at + 10)?, u32_at(at + 20)?, u32_at(at + 24)?);
        let (name_len, extra_len, comment_len, local) = (u16_at(at + 28)?, u16_at(at + 30)?, u16_at(at + 32)?, u32_at(at + 42)?);
        let name = std::str::from_utf8(data.get(at + 46..at + 46 + name_len).ok_or_else(invalid)?).map_err(|_| invalid())?.to_string();
        at += 46 + name_len + extra_len + comment_len;
        if name.ends_with('/') || !wanted(&name) { continue; }
        let relative = Path::new(&name);
        if relative.components().any(|c| !matches!(c, Component::Normal(_))) || size > 512 * 1024 * 1024 {
            return Err(io::Error::other("Caminho de pacote inválido"));
        }
        if u32_at(local)? != 0x0403_4b50 { return Err(invalid()); }
        let start = local + 30 + u16_at(local + 26)? + u16_at(local + 28)?;
        let raw = data.get(start..start + packed).ok_or_else(invalid)?;
        let mut bytes = Vec::with_capacity(size);
        match method {
            0 => bytes.extend_from_slice(raw),
            8 => { flate2::read::DeflateDecoder::new(raw).take(size as u64 + 1).read_to_end(&mut bytes)?; }
            _ => return Err(io::Error::other("Compressão do pacote não suportada")),
        }
        if bytes.len() != size { return Err(invalid()); }
        let target = destination.join(relative);
        if let Some(parent) = target.parent() { fs::create_dir_all(parent)?; }
        fs::write(&target, &bytes)?;
        if !name.contains('/') { fs::set_permissions(&target, fs::Permissions::from_mode(0o755))?; }
    }
    Ok(())
}

/// Do pacote do Real-ESRGAN, só o executável e os modelos de foto e de anime (sem o vídeo e as imagens de exemplo).
fn upscale_file(name: &str) -> bool {
    name == "realesrgan-ncnn-vulkan" || name.starts_with("models/realesrgan-x4plus")
}

/// Binário único compactado com gzip (ex.: ffmpeg estático): descompacta e torna executável.
fn unpack_binary(archive: &Path, destination: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut reader = flate2::read::GzDecoder::new(fs::File::open(archive)?).take(512 * 1024 * 1024);
    let mut file = fs::File::create(destination)?;
    io::copy(&mut reader, &mut file)?;
    file.sync_all()?;
    fs::set_permissions(destination, fs::Permissions::from_mode(0o755))
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
        let single = artifact.url.ends_with(".gz") && !artifact.url.ends_with(".tar.gz");
        let zip = artifact.url.ends_with(".zip");
        let binary = artifact.binary;
        tokio::task::spawn_blocking(move || if zip { extract_zip(&source, &dest, upscale_file) } else if single { unpack_binary(&source, &dest.join(binary)) } else { extract(&source, &dest, &cancellation) })
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
            let name = if file.projector { "projector.gguf" } else { weights_name };
            let path = staging.join(name);
            download(file.url, file.bytes, file.sha256, &path, cancel, |received| {
                progress(done + received)
            })
            .await?;
            done += file.bytes;
        }
        if cancel.is_cancelled() {
            return Err("Download interrompido".to_string());
        }
        let path = target.join(weights_name);
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
    /// Contexto do worker: embeddings usam 2048; o automático sobe para 32K quando o modelo é do
    /// catálogo (consumo medido) e roda numa placa de vídeo com folga.
    pub fn context_for(&self, model: &Model, prefs: &Preferences) -> u32 {
        if model.capability == Capability::Embedding { return 2048; }
        let gpu = || find_binary(&self.root.join("llama-vulkan"), "llama-server").is_some()
            && hardware::detect().gpus.iter().filter_map(|g| g.vram_total_bytes).max().unwrap_or(0) >= 8 << 30;
        auto_context(prefs.context_is_auto(), model.memory_bytes.is_some(), gpu).unwrap_or(prefs.context_tokens)
    }
    /// Workers que sobraram de uma Ayla que caiu sem encerrar (ex.: `kill -9`, travamento): na
    /// abertura da instância principal nenhum outro processo pode estar usando estes executáveis,
    /// então qualquer um que rode a partir da pasta de runtimes é sobra e ocupa RAM/VRAM à toa.
    pub fn reap_orphans(&self) -> usize {
        let Ok(root) = self.root.canonicalize() else { return 0 };
        let me = std::process::id();
        let mut stopped = 0;
        for entry in fs::read_dir("/proc").into_iter().flatten().flatten() {
            let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
            if pid == me { continue; }
            let Ok(exe) = fs::read_link(entry.path().join("exe")) else { continue };
            if !exe.starts_with(&root) { continue; }
            if std::process::Command::new("kill").arg(pid.to_string()).status().is_ok_and(|s| s.success()) {
                stopped += 1;
            }
        }
        stopped
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
        let context = self.context_for(model, prefs);
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
            // A placa de vídeo fica para o modelo, menos a reserva do sistema; o perfil limita a RAM.
            let margin = super::models::GPU_RESERVE / (1024 * 1024);
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
        // MTP do próprio modelo: 2 tokens por passo deram +43% de geração (benchmark de 23/11/2026).
        if !memory && model.mtp() {
            command.args(["--spec-type", "draft-mtp", "--spec-draft-n-max", "2"]);
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
    /// Transcrição (whisper.cpp) em processo próprio, sem descarregar o modelo
    /// de conversa. Grava `<output>.txt`. Cancelar encerra só este processo.
    pub async fn transcribe(
        &self,
        model: &Model,
        input: &Path,
        output: &Path,
        cancel: &CancellationToken,
    ) -> Result<(), String> {
        if !model.supports(Capability::Transcription) {
            return Err("Este modelo não faz transcrição".into());
        }
        let mut command = self.command("whisper", "whisper-cli")?;
        let threads = hardware::detect().logical_cpus.saturating_sub(1).max(1);
        command
            .args(["-m"])
            .arg(&model.path)
            .args(["-f"])
            .arg(input)
            .args(["-l", "auto", "-nt", "-np", "-t", &threads.to_string(), "-otxt", "-of"])
            .arg(output);
        let mut child = command
            .spawn()
            .map_err(|_| "Não foi possível iniciar a transcrição")?;
        let start = Instant::now();
        loop {
            if cancel.is_cancelled() || start.elapsed() > Duration::from_secs(900) {
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
impl Runtime {
    /// Caminho de um motor instalado (ex.: ffmpeg), se houver.
    pub fn tool(&self, component: &str, binary: &str) -> Option<PathBuf> {
        find_binary(&self.root.join(component), binary)
    }
}

/// Roda uma ferramenta local (ex.: ffmpeg) com limite de tempo e cancelamento;
/// devolve a saída de erro, onde o ffmpeg informa duração e problemas.
pub async fn run_tool(mut command: Command, cancel: &CancellationToken, limit: Duration) -> Result<(bool, String), String> {
    command.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());
    let mut child = command.spawn().map_err(|_| "Não foi possível iniciar o leitor de arquivos")?;
    let mut stderr = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(pipe) = stderr.as_mut() {
            let _ = pipe.take(256 * 1024).read_to_string(&mut text);
        }
        text
    });
    let start = Instant::now();
    loop {
        if cancel.is_cancelled() || start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return Err(if cancel.is_cancelled() { "Tarefa interrompida".into() } else { "O arquivo demorou demais para ser lido".into() });
        }
        if let Some(status) = child.try_wait().map_err(|_| "Falha no leitor de arquivos")? {
            return Ok((status.success(), reader.join().unwrap_or_default()));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// 32K só no automático, para modelo com consumo medido e com placa de vídeo (avaliada por último).
fn auto_context(auto: bool, measured: bool, gpu: impl FnOnce() -> bool) -> Option<u32> {
    (auto && measured && gpu()).then_some(super::preferences::GPU_CONTEXT)
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
            // llama.cpp manda no último trecho os tempos de leitura do prompt e de geração.
            if event["timings"].is_object() {
                report(json!({"type":"timings","timings":event["timings"]}));
            }
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
    /// Monta um .zip (deflate ou guardado) para testar a extração.
    fn zip_of(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
        use std::io::Write;
        let (mut out, mut central) = (Vec::new(), Vec::new());
        for (name, data, deflate) in entries {
            let packed = if *deflate {
                let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                encoder.write_all(data).unwrap();
                encoder.finish().unwrap()
            } else { data.to_vec() };
            let method: u16 = if *deflate { 8 } else { 0 };
            let offset = out.len() as u32;
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&[20, 0, 0, 0]);
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0; 8]);
            out.extend_from_slice(&(packed.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&packed);
            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            central.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&[0; 8]);
            central.extend_from_slice(&(packed.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 12]);
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let start = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&start.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out
    }

    /// Instala o ampliador a partir do pacote real (cache de desenvolvimento) e amplia uma imagem.
    #[test]
    #[ignore = "precisa do pacote em AYLA_MODEL_CACHE e de Vulkan"]
    fn instala_o_ampliador_e_amplia() {
        let root = std::env::temp_dir().join(format!("ayla-upscale-install-{}", super::super::store::id()));
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(install(&root, "upscale", &CancellationToken::new(), |_| {})).unwrap();
        let binary = find_binary(&root.join("upscale"), "realesrgan-ncnn-vulkan").expect("executável");
        let mut models: Vec<String> = fs::read_dir(root.join("upscale/models")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        models.sort();
        assert_eq!(models, ["realesrgan-x4plus-anime.bin", "realesrgan-x4plus-anime.param", "realesrgan-x4plus.bin", "realesrgan-x4plus.param"]);
        assert!(!root.join("upscale/onepiece_demo.mp4").exists());
        let input = root.join("in.png");
        let image = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 80, 60).unwrap();
        image.fill(0xcc6633ff);
        image.savev(&input, "png", &[]).unwrap();
        let output = root.join("out.png");
        super::super::upscale::run(&binary, &binary.parent().unwrap().join("models"), &input, &output, "realesrgan-x4plus-anime", &CancellationToken::new(), |_| {}).unwrap();
        assert_eq!(super::super::upscale::dimensions(&output), Some((320, 240)));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn zip_extrai_so_o_pedido_e_recusa_caminho_para_fora() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("ayla-zip-{}", super::super::store::id()));
        fs::create_dir_all(dir.join("saida")).unwrap();
        let model = vec![7u8; 5000];
        let archive = dir.join("pacote.zip");
        fs::write(&archive, zip_of(&[("realesrgan-ncnn-vulkan", b"#!binario", false), ("models/realesrgan-x4plus.bin", &model, true),
            ("models/realesr-animevideov3-x2.bin", b"fora", true), ("onepiece_demo.mp4", b"video", false)])).unwrap();
        extract_zip(&archive, &dir.join("saida"), upscale_file).unwrap();
        assert_eq!(fs::read(dir.join("saida/models/realesrgan-x4plus.bin")).unwrap(), model, "deflate");
        let binary = dir.join("saida/realesrgan-ncnn-vulkan");
        assert_eq!(fs::read(&binary).unwrap(), b"#!binario");
        assert_eq!(fs::metadata(&binary).unwrap().permissions().mode() & 0o111, 0o111, "executável");
        assert!(!dir.join("saida/onepiece_demo.mp4").exists() && !dir.join("saida/models/realesr-animevideov3-x2.bin").exists());
        let evil = dir.join("mau.zip");
        fs::write(&evil, zip_of(&[("models/realesrgan-x4plus/../../fora", b"x", false)])).unwrap();
        assert!(extract_zip(&evil, &dir.join("saida"), upscale_file).is_err());
        assert!(!dir.join("fora").exists());
        let _ = fs::remove_dir_all(dir);
    }

    use super::*;

    #[test]
    fn automatic_context_needs_a_measured_model_and_a_gpu() {
        assert_eq!(auto_context(true, true, || true), Some(32768));
        assert_eq!(auto_context(true, true, || false), None);
        assert_eq!(auto_context(true, false, || panic!("modelo importado nem consulta a GPU")), None);
        assert_eq!(auto_context(false, true, || true), None);
    }

    #[test]
    fn orphans_from_the_runtimes_folder_are_stopped_and_others_are_kept() {
        let root = std::env::temp_dir().join(format!("ayla-reap-{}", std::process::id()));
        let bin = root.join("llama-cpu");
        fs::create_dir_all(&bin).unwrap();
        let fake = bin.join("llama-server");
        fs::copy("/usr/bin/sleep", &fake).unwrap();
        let mut orphan = std::process::Command::new(&fake).arg("60").spawn().unwrap();
        let mut other = std::process::Command::new("/usr/bin/sleep").arg("60").spawn().unwrap();
        // Espera o exec trocar a imagem do processo.
        while fs::read_link(format!("/proc/{}/exe", orphan.id())).map_or(true, |e| !e.starts_with(&bin)) { std::thread::sleep(Duration::from_millis(5)); }
        let runtime = Runtime::new(root.clone());
        assert_eq!(runtime.reap_orphans(), 1);
        assert!(orphan.wait().unwrap().code().is_none(), "encerrado por sinal");
        assert!(other.try_wait().unwrap().is_none(), "processo de fora continua");
        other.kill().unwrap();
        let _ = other.wait();
        fs::remove_dir_all(root).unwrap();
    }
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
            gpu_bytes: 0,
            gpu_only: false,
            mtp: false,
            files: Box::leak(Box::new([
                super::super::models::CatalogFile { url: url("/w"), bytes: 7, sha256: digest(b"weights"), projector: false },
                super::super::models::CatalogFile { url: url("/p"), bytes: 4, sha256: projector_sha, projector: true },
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
mod gpu_tests {
    use super::*;
    use base64::Engine;
    /// Rode com AYLA_MODEL_CACHE=/var/tmp/ayla-benchmark: instala o Qwen3.6 35B do catálogo pelo cache
    /// (tamanho e SHA-256 conferidos) e conversa pelo runtime Vulkan já instalado, com MTP e imagem.
    #[tokio::test]
    #[ignore = "modelo de 12,6 GB e placa de vídeo reais"]
    async fn catalog_moe_runs_on_the_gpu_with_mtp_vision_and_timings() {
        let root = std::env::temp_dir().join(format!("ayla-gpu-real-{}", super::super::store::id()));
        fs::create_dir_all(root.join("runtimes")).unwrap();
        let home = std::env::var("HOME").unwrap();
        std::os::unix::fs::symlink(format!("{home}/.local/share/dev.feit.ayla/runtimes/llama-vulkan"), root.join("runtimes").join("llama-vulkan")).unwrap();
        let entry = super::super::models::catalog().iter().find(|e| e.id == "qwen3.6-35b-a3b").unwrap();
        let token = CancellationToken::new();
        let model = install_model(&root.join("models"), entry, &token, |_| {}).await.unwrap();
        assert!(model.mtp() && model.companion.is_some() && model.memory_bytes.is_some());
        let runtime = Runtime::new(root.join("runtimes"));
        let prefs = Preferences::default();
        assert_eq!(runtime.context_for(&model, &prefs), 32768, "contexto automático com GPU");
        let image = fs::read("/tmp/claude-1000/-home-feit-Documents-programming-ayla/f52c5660-6ce4-448b-8115-42b55203c568/scratchpad/img.png").unwrap();
        let url = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(image));
        let messages = vec![json!({"role":"user","content":[{"type":"text","text":"De que obra é esta capa? Responda em uma frase."},{"type":"image_url","image_url":{"url":url}}]})];
        let mut timings = Value::Null;
        let start = Instant::now();
        let (text, first, _) = runtime.local_chat(&model, Capability::Vision, &prefs, messages, &token, |e| if e["type"] == "timings" { timings = e["timings"].clone(); }).await.unwrap();
        eprintln!("{} ms até o 1º token, {} ms no total; resposta: {text}; tempos: {timings}", first.unwrap_or(0), start.elapsed().as_millis());
        assert!(text.contains("One Piece"), "{text}");
        assert!(timings["predicted_per_second"].as_f64().unwrap() > 0.0);
        assert!(timings["draft_n"].as_u64().unwrap_or(0) > 0 || timings.get("draft_n_accepted").is_some(), "MTP ativo: {timings}");
        runtime.stop_all();
        drop(runtime);
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod voice_tests {
    use super::*;
    /// Rode com AYLA_MODEL_CACHE=/var/tmp/ayla-benchmark (arquivos já verificados; sem rede).
    /// Usa um trecho real do FLEURS pt_BR (CC-BY-4.0) já presente no cache do benchmark.
    #[tokio::test]
    #[ignore = "modelos de transcrição reais via cache local; motor de /tmp/ayla-p1-runtime-validation"]
    async fn catalog_transcribers_install_and_transcribe_real_speech() {
        let sample = Path::new("/var/tmp/ayla-benchmark/fleurs/pcm16/7377029495898915434.wav");
        let root = std::env::temp_dir().join(format!("ayla-voice-real-{}", super::super::store::id()));
        fs::create_dir_all(root.join("runtimes")).unwrap();
        std::os::unix::fs::symlink("/tmp/ayla-p1-runtime-validation/whisper", root.join("runtimes").join("whisper")).unwrap();
        let entry = |id: &str| super::super::models::catalog().iter().find(|e| e.id == id).unwrap();
        let token = CancellationToken::new();
        let runtime = Runtime::new(root.join("runtimes"));
        for id in ["whisper-small", "whisper-tiny"] {
            let model = install_model(&root.join("models"), entry(id), &token, |_| {}).await.unwrap();
            let start = Instant::now();
            runtime.transcribe(&model, sample, &root.join(id), &token).await.unwrap();
            let heard = fs::read_to_string(root.join(id).with_extension("txt")).unwrap().to_lowercase();
            eprintln!("{id}: {} ms → {heard}", start.elapsed().as_millis());
            // O tiny (prévia) erra nomes próprios; as duas devem acertar o essencial do trecho.
            assert!(heard.contains("presença militar") && heard.contains("acordo"), "a transcrição deve reconhecer o trecho");
        }
        assert!(runtime.status()["active_model"].is_null(), "transcrição não usa o worker de conversa");
        fs::remove_dir_all(root).unwrap();
    }
}
