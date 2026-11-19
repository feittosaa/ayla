use super::{hardware, models::Capability, models::Model, preferences::Preferences};
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
    let result=async {
        let client=reqwest::Client::builder().connect_timeout(Duration::from_secs(15)).read_timeout(Duration::from_secs(60)).build().map_err(|_|"Falha ao preparar download")?;
        let response=tokio::select!{ _=cancel.cancelled()=>return Err("Instalação interrompida".to_string()), response=client.get(artifact.url).send()=>response.map_err(|_|"Não foi possível baixar o componente")? };
        let response=response.error_for_status().map_err(|_|"O servidor não disponibilizou este componente")?;
        let mut file=tokio::fs::File::create(&temporary).await.map_err(|_|"Não foi possível salvar download")?;
        let mut stream=response.bytes_stream(); let mut sha=Sha256::new(); let mut received=0u64;
        loop {
            let item=tokio::select!{_=cancel.cancelled()=>return Err("Instalação interrompida".to_string()),item=stream.next()=>item};
            let Some(item)=item else{break;}; let bytes=item.map_err(|_|"Download interrompido")?;
            received+=bytes.len() as u64;if received>artifact.bytes{return Err("Componente com tamanho inesperado".into());}
            sha.update(&bytes);file.write_all(&bytes).await.map_err(|_|"Sem espaço para salvar componente")?;
            report(json!({"type":"download","received":received,"total":artifact.bytes}));
        }
        file.sync_all().await.map_err(|_|"Não foi possível concluir o download")?;drop(file);
        if received!=artifact.bytes || format!("{:x}",sha.finalize())!=artifact.sha256{return Err("O componente não passou na verificação de integridade".into());}
        let _=fs::remove_dir_all(&staging);fs::create_dir_all(&staging).map_err(|_|"Não foi possível preparar componente")?;
        let source=temporary.clone();let dest=staging.clone();let cancellation=cancel.clone();
        tokio::task::spawn_blocking(move||extract(&source,&dest,artifact.url.ends_with(".bz2"),&cancellation)).await.map_err(|_|"Falha ao extrair componente")?.map_err(|_|"Pacote inválido ou instalação interrompida")?;
        if find_binary(&staging,artifact.binary).is_none(){return Err("O pacote não inclui o componente esperado".into());}
        if cancel.is_cancelled(){return Err("Instalação interrompida".into());}
        fs::rename(&staging,&target).map_err(|_|"Não foi possível concluir a instalação")?;
        Ok(())
    }.await;
    let _ = fs::remove_file(&temporary);
    if result.is_err() {
        let _ = fs::remove_dir_all(staging);
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
    pub gate: tokio::sync::Mutex<()>,
    client: reqwest::Client,
}
impl Runtime {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            worker: Mutex::new(None),
            gate: tokio::sync::Mutex::new(()),
            client: reqwest::Client::builder()
                .no_proxy()
                .connect_timeout(Duration::from_secs(3))
                .read_timeout(Duration::from_secs(120))
                .build()
                .unwrap(),
        }
    }
    pub fn stop(&self) {
        if let Ok(mut worker) = self.worker.lock() {
            if let Some(mut worker) = worker.take() {
                let _ = worker.child.kill();
                let _ = worker.child.wait();
            }
        }
    }
    pub fn idle_check(&self) {
        let idle = self
            .worker
            .lock()
            .ok()
            .and_then(|w| {
                w.as_ref()
                    .map(|w| w.last_used.elapsed() > Duration::from_secs(w.idle_seconds))
            })
            .unwrap_or(false);
        if idle {
            if let Ok(_guard) = self.gate.try_lock() {
                self.stop();
            }
        }
    }
    pub fn resident_bytes(&self) -> u64 {
        self.worker
            .lock()
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
    pub fn status(&self) -> Value {
        let active = self
            .worker
            .lock()
            .ok()
            .and_then(|w| w.as_ref().map(|w| w.model_id.clone()));
        let resident = self.resident_bytes();
        json!({"active_model":active,"resident_bytes":resident})
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
        {
            let mut guard = self.worker.lock().map_err(|_| "Modelo ocupado")?;
            if let Some(worker) = guard.as_mut() {
                if worker.model_id == model.id
                    && worker.context == prefs.context_tokens
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
        self.stop();
        let gpu = self.root.join("llama-vulkan");
        let component = if find_binary(&gpu, "llama-server").is_some() {
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
            &prefs.context_tokens.to_string(),
            "--parallel",
            "1",
            "--threads",
            &hardware::detect()
                .logical_cpus
                .saturating_sub(1)
                .max(1)
                .to_string(),
            "--no-warmup",
            "--offline",
            "--no-webui",
            "--no-slots",
        ]);
        command.env("LLAMA_API_KEY", &token);
        if component == "llama-cpu" {
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
            command.arg("--embedding");
        }
        let child=command.spawn().map_err(|_|"Não foi possível executar o modelo. Verifique a compatibilidade do componente com seu Linux.")?;
        let url = format!("http://127.0.0.1:{port}");
        *self.worker.lock().map_err(|_| "Modelo ocupado")? = Some(Worker {
            child,
            model_id: model.id.clone(),
            url: url.clone(),
            token: token.clone(),
            last_used: Instant::now(),
            idle_seconds: prefs.idle_seconds,
            context: prefs.context_tokens,
        });
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(120) {
            if cancel.is_cancelled() {
                self.stop();
                return Err("Carregamento interrompido".into());
            }
            let dead = self
                .worker
                .lock()
                .map_err(|_| "Modelo ocupado")?
                .as_mut()
                .and_then(|w| w.child.try_wait().ok().flatten())
                .is_some();
            if dead {
                self.stop();
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
            tokio::select! {_=cancel.cancelled()=>{self.stop();return Err("Carregamento interrompido".into());},_=tokio::time::sleep(Duration::from_millis(100))=>{}}
        }
        self.stop();
        Err("Tempo de carregamento excedido".into())
    }
    pub async fn local_chat(
        &self,
        model: &Model,
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
            .json(&json!({"messages":messages,"stream":true,"max_tokens":prefs.context_tokens/2}));
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
        let (url, token) = self.ensure(model, prefs, cancel).await?;
        let response = tokio::select! {_=cancel.cancelled()=>{self.stop();return Err("Tarefa interrompida".into());},r=self.client.post(format!("{url}/v1/embeddings")).bearer_auth(token).json(&json!({"input":text})).send()=>r.map_err(|_|"Falha no modelo de memória")?};
        let data: Value = response
            .error_for_status()
            .map_err(|_| "Modelo incompatível com embeddings")?
            .json()
            .await
            .map_err(|_| "Resposta de memória inválida")?;
        let vector: Vec<f32> = serde_json::from_value(data["data"][0]["embedding"].clone())
            .map_err(|_| "Vetor inválido")?;
        if vector.is_empty() || vector.iter().any(|n| !n.is_finite()) {
            return Err("Vetor inválido".into());
        }
        Ok(vector)
    }
    pub async fn voice(
        &self,
        model: &Model,
        input: &Path,
        text: &str,
        output: &Path,
        cancel: &CancellationToken,
    ) -> Result<(), String> {
        self.stop();
        let mut command = if model.capability == Capability::Transcription {
            let mut c = self.command("whisper", "whisper-cli")?;
            c.args(["-m"])
                .arg(&model.path)
                .args(["-f"])
                .arg(input)
                .args(["-l", "auto", "-otxt", "-of"])
                .arg(output);
            c
        } else {
            let library = find_binary(&self.root.join("speech"), "libsherpa-onnx-c-api.so")
                .ok_or("Instale o componente de síntese de voz")?;
            let project = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .ok_or("Pasta da aplicação inválida")?;
            let python = std::env::var_os("AYLA_PYTHON")
                .map(PathBuf::from)
                .unwrap_or_else(|| project.join(".venv/bin/python"));
            let mut c = Command::new(python);
            c.arg(project.join("backend/adapters/speech.py"))
                .env("LD_LIBRARY_PATH", library.parent().unwrap());
            c.stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            c
        };
        let mut child = command
            .spawn()
            .map_err(|_| "Não foi possível iniciar componente de voz")?;
        if model.capability == Capability::Speech {
            use std::io::Write;
            let library = find_binary(&self.root.join("speech"), "libsherpa-onnx-c-api.so")
                .ok_or("Componente de voz incompleto")?;
            let request = json!({"library":library,"model":model.path,"text":text,"output":output,"threads":hardware::detect().logical_cpus.saturating_sub(1).max(1)});
            if child
                .stdin
                .take()
                .ok_or("Entrada de voz indisponível")?
                .write_all(request.to_string().as_bytes())
                .is_err()
            {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Falha ao enviar texto ao componente de voz".into());
            }
        }
        *self.worker.lock().map_err(|_| "Modelo ocupado")? = Some(Worker {
            child,
            model_id: model.id.clone(),
            url: String::new(),
            token: String::new(),
            last_used: Instant::now(),
            idle_seconds: 180,
            context: 0,
        });
        let start = Instant::now();
        loop {
            if cancel.is_cancelled() || start.elapsed() > Duration::from_secs(180) {
                self.stop();
                return Err("Tarefa de voz interrompida".into());
            }
            let status = {
                let mut guard = self.worker.lock().map_err(|_| "Modelo ocupado")?;
                guard
                    .as_mut()
                    .ok_or("Tarefa interrompida")?
                    .child
                    .try_wait()
                    .map_err(|_| "Falha no componente de voz")?
            };
            if let Some(status) = status {
                self.stop();
                return if status.success() {
                    Ok(())
                } else {
                    Err("O componente de voz não conseguiu usar este modelo ou arquivo".into())
                };
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.stop();
    }
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
            path: root.join(file),
            companion: None,
            bytes: fs::metadata(root.join(file)).unwrap().len(),
            sha256: String::new(),
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
        let result = runtime.local_chat(&vision, &prefs, vec![json!({"role":"user","content":[{"type":"text","text":"Describe this image briefly."},{"type":"image_url","image_url":{"url":format!("data:image/png;base64,{image}")}}]})], &token, |_| {}).await.unwrap();
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
