//! Órgãos: produtos separados que a pessoa escolhe baixar junto da Ayla.
//!
//! A Moneta é o primeiro. A Ayla não copia o código dela: baixa o repositório oficial na versão
//! fixada em `organs/moneta/organ.json`, confere o commit e sobe o `compose.yaml` do próprio
//! repositório num projeto Docker separado, com a sobreposição `compose.ayla.yaml`. A interface
//! da Moneta aparece dentro da janela da Ayla pelo protocolo `moneta://`, que repassa cada pedido
//! ao servidor local da Moneta com a chave de acesso (contrato em `docs/integracao.md` da Moneta).
//! A Moneta instalada separadamente continua independente, com dados e porta próprios: ela lacra
//! os segredos com o `systemd-creds` do computador, que o contêiner não consegue abrir.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};

const COMPOSE_OVERRIDE: &str = include_str!("../../../../organs/moneta/compose.ayla.yaml");
/// Variáveis visuais da Ayla (fonte única: o tema da interface).
const AYLA_THEME: &str = include_str!("../../../src/styles/ayla-theme.css");
/// Como as variáveis do órgão seguem as da Ayla: fundos, superfícies, texto e fontes.
const ORGAN_THEME: &str = include_str!("../../../../organs/moneta/tema.css");
/// Caminho reservado à Ayla dentro do protocolo do órgão.
pub const THEME_PATH: &str = "/__ayla/tema.css";

/// Folha servida em `THEME_PATH`: tema da Ayla e, depois, o mapeamento do órgão.
pub fn theme_css() -> String {
    format!("{AYLA_THEME}\n{ORGAN_THEME}")
}

/// Inclui o tema da Ayla no fim do `<head>` de uma página do órgão, depois das folhas dele, para
/// que fundos e fontes sigam a Ayla. Páginas sem `<head>` recebem a folha no início.
pub fn with_theme(html: &str) -> String {
    let link = format!("<link rel=\"stylesheet\" href=\"{THEME_PATH}\">");
    match html.to_ascii_lowercase().rfind("</head>") {
        Some(at) => format!("{}{link}{}", &html[..at], &html[at..]),
        None => format!("{link}{html}"),
    }
}
const MANIFEST: &str = include_str!("../../../../organs/moneta/organ.json");
/// Ícone da Moneta (o mesmo do repositório dela), para o trilho mostrar o órgão antes do download.
const ICON: &[u8] = include_bytes!("../../../../organs/moneta/icon.png");
/// Projeto do Docker Compose da Moneta dentro da Ayla, separado de qualquer outro.
const PROJECT: &str = "ayla-moneta";

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub description: String,
    pub repository: String,
    pub tag: String,
    pub commit: String,
    pub contract: u32,
    pub service: String,
    pub port: u16,
    pub compose: String,
    /// Executável da Moneta instalada separadamente, relativo à pasta pessoal.
    pub standalone: String,
    /// Porta e serviço de usuário (systemd) da Moneta instalada.
    pub standalone_port: u16,
    pub standalone_service: String,
}

pub fn manifest() -> Manifest {
    serde_json::from_str(MANIFEST).expect("organ.json da Moneta inválido")
}

/// Resposta pública de `GET /api/status` (contrato da Moneta, versão 1).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Health {
    pub api: u32,
    pub instance: String,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub lock: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    /// Repositório baixado e conferido nesta instalação da Ayla.
    pub installed: bool,
    /// Etapa em andamento (baixar, preparar, iniciar), para a interface acompanhar.
    pub task: Option<String>,
    pub error: Option<String>,
    /// Docker com Compose disponível para o usuário.
    pub engine: bool,
    /// Servidor da Moneta da Ayla respondendo.
    pub running: bool,
    /// Contêiner da Ayla em execução.
    pub container: bool,
    pub locked: bool,
    pub contract_ok: bool,
    /// Moneta instalada neste computador: a Ayla usa essa mesma Moneta (mesmos dados).
    pub standalone: bool,
    /// Repositório baixado para o contêiner.
    pub downloaded: bool,
    /// Ícone do órgão (data URL), disponível mesmo antes do download.
    pub icon: Option<String>,
}

#[derive(Default)]
struct State {
    task: Option<String>,
    error: Option<String>,
}

pub struct Organ {
    pub manifest: Manifest,
    /// Pasta do órgão dentro dos dados da Ayla (repositório baixado e sobreposição).
    dir: PathBuf,
    data: PathBuf,
    port: u16,
    repository: String,
    state: Mutex<State>,
    client: reqwest::Client,
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// Fuso do computador (`TZ` ou o link `/etc/localtime`), para o contêiner contar os dias como a pessoa.
pub fn host_timezone() -> String {
    if let Ok(tz) = std::env::var("TZ") {
        let tz = tz.trim_start_matches(':');
        if !tz.is_empty() && !tz.starts_with('/') {
            return tz.to_string();
        }
    }
    fs::read_link("/etc/localtime")
        .ok()
        .and_then(|p| p.to_string_lossy().split_once("zoneinfo/").map(|(_, zone)| zone.to_string()))
        .filter(|zone| !zone.is_empty() && zone.bytes().all(|b| b.is_ascii_alphanumeric() || b"/_-+".contains(&b)))
        .unwrap_or_else(|| "UTC".into())
}

pub fn valid_key(text: &str) -> Option<String> {
    let key = text.trim();
    (key.len() == 43 && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
        .then(|| key.to_string())
}

/// Troca `frame-ancestors 'none'` pela janela da Ayla: é o que permite mostrar a Moneta na mesma
/// tela. O resto da política de segurança da Moneta fica como está.
pub fn embeddable_csp(policy: &str) -> String {
    policy
        .split(';')
        .map(str::trim)
        .filter(|d| !d.is_empty() && !d.starts_with("frame-ancestors"))
        .chain(std::iter::once("frame-ancestors 'self' tauri://localhost http://tauri.localhost http://localhost:5173"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Caminho pedido pela página; recusa o que não for caminho absoluto simples e a entrada por URL
/// com chave, que a Ayla não usa (ela envia a chave no cabeçalho).
pub fn safe_path(path_and_query: &str) -> Option<&str> {
    let ok = path_and_query.starts_with('/')
        && !path_and_query.starts_with("//")
        && !path_and_query.starts_with("/entrar")
        && !path_and_query.bytes().any(|b| b.is_ascii_control() || b == b' ');
    ok.then_some(path_and_query)
}

impl Organ {
    pub fn moneta(root: &Path) -> Self {
        let manifest = manifest();
        let port = std::env::var("AYLA_MONETA_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(manifest.port);
        // Fora da pasta do órgão: remover a Moneta baixada nunca apaga os dados dela.
        let data = std::env::var_os("AYLA_MONETA_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("organ-data").join(&manifest.id));
        // Desenvolvimento e testes podem apontar outro clone (por exemplo ../moneta).
        let repository = std::env::var("AYLA_MONETA_REPO").unwrap_or_else(|_| manifest.repository.clone());
        Self {
            dir: root.join("organs").join(&manifest.id),
            manifest,
            data,
            port,
            repository,
            state: Mutex::new(State::default()),
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(120))
                .build()
                .expect("cliente HTTP"),
        }
    }

    fn source(&self) -> PathBuf {
        self.dir.join("source")
    }
    fn marker(&self) -> PathBuf {
        self.dir.join("instalado")
    }
    /// Relação simbiótica: com a Moneta instalada neste computador, a Ayla usa essa mesma Moneta
    /// (mesmos dados, mesmo cofre); o contêiner é só para quem não a tem instalada.
    /// `AYLA_MONETA_STANDALONE=0` força o contêiner (testes).
    pub fn uses_standalone(&self) -> bool {
        std::env::var("AYLA_MONETA_STANDALONE").as_deref() != Ok("0") && self.standalone().is_some()
    }
    fn port(&self) -> u16 {
        if self.uses_standalone() { self.manifest.standalone_port } else { self.port }
    }
    fn data_dir(&self) -> PathBuf {
        if self.uses_standalone() {
            // Mesma pasta que a Moneta usa no modo de desktop (server.mjs --desktop).
            std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
                .unwrap_or_else(|| home().join(".local/share")).join(&self.manifest.id)
        } else {
            self.data.clone()
        }
    }
    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port())
    }

    /// Disponível na Ayla: instalada neste computador ou baixada para o contêiner.
    pub fn installed(&self) -> bool {
        self.uses_standalone() || self.downloaded()
    }

    /// Repositório baixado para o contêiner, no commit fixado.
    pub fn downloaded(&self) -> bool {
        fs::read_to_string(self.marker()).is_ok_and(|c| c.trim() == self.manifest.commit)
            && self.source().join(&self.manifest.compose).is_file()
    }

    pub fn standalone(&self) -> Option<PathBuf> {
        let path = home().join(&self.manifest.standalone);
        path.is_file().then_some(path)
    }

    fn set_task(&self, task: Option<&str>) {
        if let Ok(mut state) = self.state.lock() {
            state.task = task.map(str::to_string);
            if task.is_some() {
                state.error = None;
            }
        }
    }
    fn set_error(&self, error: &str) {
        if let Ok(mut state) = self.state.lock() {
            state.task = None;
            state.error = Some(error.to_string());
        }
    }
    /// Reserva a etapa; recusa se outra já estiver em andamento.
    fn begin(&self, task: &str) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|_| "Órgão ocupado")?;
        if let Some(current) = &state.task {
            return Err(format!("A Moneta já está em outra etapa: {current}"));
        }
        state.task = Some(task.to_string());
        state.error = None;
        Ok(())
    }

    pub fn access_key(&self) -> Option<String> {
        valid_key(&fs::read_to_string(self.data_dir().join("acesso.key")).ok()?)
    }

    pub async fn health(&self) -> Option<Health> {
        let response = self
            .client
            .get(format!("{}/api/status", self.base()))
            .timeout(Duration::from_millis(1500))
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        response.json::<Health>().await.ok()
    }

    fn compose(&self) -> Command {
        let mut command = Command::new("docker");
        command
            .arg("compose")
            .arg("--project-name")
            .arg(PROJECT)
            .arg("--file")
            .arg(self.source().join(&self.manifest.compose))
            .arg("--file")
            .arg(self.dir.join("compose.ayla.yaml"))
            .current_dir(self.source())
            .env("AYLA_MONETA_DATA", &self.data)
            .env("PORT", self.port.to_string())
            .env("AYLA_TZ", host_timezone())
            // As credenciais da Pluggy ficam no cofre da Moneta; nada do ambiente da Ayla vaza.
            .env_remove("PLUGGY_CLIENT_ID")
            .env_remove("PLUGGY_CLIENT_SECRET")
            .stdin(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if let Ok(meta) = fs::metadata(&self.data) {
                command.env("MONETA_UID", meta.uid().to_string()).env("MONETA_GID", meta.gid().to_string());
            }
        }
        command
    }

    pub fn engine() -> bool {
        Command::new("docker")
            .args(["compose", "version"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    /// Contêiner da Ayla em execução.
    pub fn container(&self) -> bool {
        if !self.downloaded() || self.uses_standalone() {
            return false;
        }
        self.compose()
            .args(["ps", "--status", "running", "--quiet", &self.manifest.service])
            .stderr(Stdio::null())
            .output()
            .is_ok_and(|o| o.status.success() && !o.stdout.trim_ascii().is_empty())
    }

    fn run(&self, command: &mut Command, failure: &str) -> Result<(), String> {
        let output = command.stdout(Stdio::null()).output().map_err(|_| failure.to_string())?;
        if output.status.success() {
            return Ok(());
        }
        let detail = String::from_utf8_lossy(&output.stderr);
        let last = detail.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        Err(if last.is_empty() { failure.to_string() } else { format!("{failure} ({last})") })
    }

    fn icon(&self) -> String {
        use base64::Engine;
        format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(ICON))
    }

    pub async fn status(&self) -> Status {
        let installed = self.installed();
        let health = self.health().await;
        let (task, error) = self.state.lock().map(|s| (s.task.clone(), s.error.clone())).unwrap_or_default();
        Status {
            id: self.manifest.id.clone(),
            name: self.manifest.name.clone(),
            description: self.manifest.description.clone(),
            version: self.manifest.tag.clone(),
            installed,
            task,
            error,
            engine: tokio::task::block_in_place(Self::engine),
            running: health.is_some(),
            container: health.is_some() && tokio::task::block_in_place(|| self.container()),
            locked: health.as_ref().is_some_and(|h| h.locked),
            contract_ok: health.as_ref().is_none_or(|h| h.api == self.manifest.contract),
            standalone: self.uses_standalone(),
            downloaded: self.downloaded(),
            icon: Some(self.icon()),
        }
    }

    /// Baixa o repositório oficial na versão fixada, confere o commit e prepara a imagem Docker.
    /// Bloqueante: chamar fora do laço assíncrono. Usado pela interface e por `ayla --instalar-moneta`.
    pub fn install(&self, progress: &dyn Fn(&str)) -> Result<(), String> {
        self.begin("Baixando a Moneta")?;
        let result = self.install_steps(progress);
        match &result {
            Ok(()) => self.set_task(None),
            Err(error) => self.set_error(error),
        }
        result
    }

    fn install_steps(&self, progress: &dyn Fn(&str)) -> Result<(), String> {
        if !Self::engine() {
            return Err("A Moneta roda num contêiner separado e precisa do Docker com Compose. Instale o Docker e tente de novo.".into());
        }
        progress("Baixando a Moneta");
        fs::create_dir_all(&self.dir).map_err(|_| "Não foi possível criar a pasta da Moneta")?;
        let partial = self.dir.join("source.partial");
        let _ = fs::remove_dir_all(&partial);
        let git = |args: &[&str], failure: &str| {
            self.run(Command::new("git").arg("-C").arg(&partial).args(args).env("GIT_TERMINAL_PROMPT", "0"), failure)
        };
        fs::create_dir_all(&partial).map_err(|_| "Não foi possível criar a pasta da Moneta")?;
        git(&["init", "--quiet"], "O Git não está disponível")?;
        let tag = format!("refs/tags/{}", self.manifest.tag);
        git(&["fetch", "--quiet", "--depth", "1", &self.repository, &tag], "Não foi possível baixar a Moneta")?;
        git(&["-c", "advice.detachedHead=false", "checkout", "--quiet", "--detach", "FETCH_HEAD"], "Não foi possível abrir a Moneta baixada")?;
        let head = Command::new("git")
            .arg("-C")
            .arg(&partial)
            .args(["rev-parse", "HEAD"])
            .output()
            .map_err(|_| "O Git não está disponível")?;
        let head = String::from_utf8_lossy(&head.stdout).trim().to_string();
        if head != self.manifest.commit {
            let _ = fs::remove_dir_all(&partial);
            return Err(format!("A versão {} baixada não confere com a esperada; nada foi instalado.", self.manifest.tag));
        }
        let _ = fs::remove_file(self.marker());
        let _ = fs::remove_dir_all(self.source());
        fs::rename(&partial, self.source()).map_err(|_| "Não foi possível guardar a Moneta baixada")?;
        fs::write(self.dir.join("compose.ayla.yaml"), COMPOSE_OVERRIDE).map_err(|_| "Não foi possível preparar a Moneta")?;
        self.prepare_data()?;
        progress("Preparando o contêiner da Moneta");
        self.set_task(Some("Preparando o contêiner da Moneta"));
        self.run(self.compose().args(["build", "--quiet", &self.manifest.service]), "Não foi possível preparar o contêiner da Moneta")?;
        fs::write(self.marker(), &self.manifest.commit).map_err(|_| "Não foi possível concluir a instalação")?;
        Ok(())
    }

    /// Pasta de dados criada antes do Docker, para não nascer com o dono errado.
    fn prepare_data(&self) -> Result<(), String> {
        fs::create_dir_all(&self.data).map_err(|_| "Não foi possível criar a pasta de dados da Moneta")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&self.data, fs::Permissions::from_mode(0o700));
        }
        Ok(())
    }

    /// Garante a Moneta da Ayla no ar: usa o contêiner se já estiver respondendo ou o sobe e espera
    /// o servidor e a chave de acesso.
    pub async fn start(&self) -> Result<Health, String> {
        if self.uses_standalone() {
            return self.start_standalone().await;
        }
        if !self.downloaded() {
            return Err("A Moneta ainda não foi baixada.".into());
        }
        if let Some(health) = self.health().await {
            if self.access_key().is_some() {
                return self.check_contract(health);
            }
            return Err(format!("Outro programa está usando a porta {} da Moneta da Ayla.", self.port));
        }
        self.begin("Iniciando a Moneta")?;
        let result = self.start_container().await;
        match &result {
            Ok(_) => self.set_task(None),
            Err(error) => self.set_error(error),
        }
        result
    }

    async fn start_container(&self) -> Result<Health, String> {
        self.prepare_data()?;
        tokio::task::block_in_place(|| {
            self.run(self.compose().args(["up", "--detach", "--no-build", &self.manifest.service]), "Não foi possível iniciar o contêiner da Moneta")
        })?;
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(90) {
            if let Some(health) = self.health().await {
                if self.access_key().is_some() {
                    return self.check_contract(health);
                }
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
        Err("A Moneta não respondeu a tempo.".into())
    }

    /// Usa a Moneta instalada: se estiver desligada, liga pelo serviço de usuário (ou pelo próprio
    /// executável, em segundo plano). A Ayla nunca a encerra.
    async fn start_standalone(&self) -> Result<Health, String> {
        if let Some(health) = self.health().await {
            return match self.access_key() {
                Some(_) => self.check_contract(health),
                None => Err("Não consegui ler a chave de acesso da Moneta.".into()),
            };
        }
        self.begin("Ligando a Moneta")?;
        let service = Command::new("systemctl")
            .args(["--user", "start", &self.manifest.standalone_service])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !service {
            if let Some(path) = self.standalone() {
                let _ = Command::new(path).arg("--background").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
            }
        }
        let started = Instant::now();
        let result = loop {
            if let Some(health) = self.health().await {
                if self.access_key().is_some() {
                    break self.check_contract(health);
                }
            }
            if started.elapsed() > Duration::from_secs(60) {
                break Err("A Moneta não respondeu a tempo.".into());
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        };
        match &result {
            Ok(_) => self.set_task(None),
            Err(error) => self.set_error(error),
        }
        result
    }

    fn check_contract(&self, health: Health) -> Result<Health, String> {
        if health.api != self.manifest.contract {
            return Err(format!(
                "Esta Moneta usa o contrato {} e a Ayla espera o {}. Atualize as duas.",
                health.api, self.manifest.contract
            ));
        }
        Ok(health)
    }

    /// Encerra o contêiner da Ayla (a Moneta grava o cofre ao receber SIGTERM). A versão instalada
    /// separadamente nunca é tocada pela Ayla.
    pub fn stop(&self, wait: bool) {
        if !self.downloaded() || self.uses_standalone() {
            return;
        }
        let mut command = self.compose();
        command.args(["stop", &self.manifest.service]).stdout(Stdio::null()).stderr(Stdio::null());
        if wait {
            let _ = command.status();
        } else {
            let _ = command.spawn();
        }
    }

    /// Remove o contêiner, a imagem e o repositório baixado. Os dados da Moneta ficam.
    pub fn remove(&self) -> Result<(), String> {
        self.begin("Removendo a Moneta")?;
        if self.downloaded() {
            let _ = self.compose().args(["down", "--rmi", "local", "--remove-orphans"]).stdout(Stdio::null()).stderr(Stdio::null()).status();
        }
        let result = fs::remove_dir_all(&self.dir).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) });
        match result {
            Ok(()) => {
                self.set_task(None);
                Ok(())
            }
            Err(_) => {
                let error = "Não foi possível remover a Moneta baixada";
                self.set_error(error);
                Err(error.into())
            }
        }
    }

    /// Repassa um pedido da página `moneta://` ao servidor local da Moneta.
    pub async fn forward(&self, request: tauri::http::Request<Vec<u8>>) -> tauri::http::Response<Vec<u8>> {
        use tauri::http::{header, Method, Response, StatusCode};
        let reply = |status: StatusCode, text: &str| {
            Response::builder()
                .status(status)
                .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .header(header::CACHE_CONTROL, "no-store")
                .body(text.as_bytes().to_vec())
                .unwrap_or_default()
        };
        let Some(path) = request.uri().path_and_query().map(|p| p.as_str()).and_then(safe_path) else {
            return reply(StatusCode::FORBIDDEN, "Endereço inválido.");
        };
        if path == THEME_PATH {
            return Response::builder()
                .header(header::CONTENT_TYPE, "text/css; charset=utf-8")
                .header(header::CACHE_CONTROL, "no-store")
                .body(theme_css().into_bytes())
                .unwrap_or_default();
        }
        let method = request.method().clone();
        if ![Method::GET, Method::HEAD, Method::POST].contains(&method) {
            return reply(StatusCode::METHOD_NOT_ALLOWED, "Método não permitido.");
        }
        let Some(key) = self.access_key() else {
            return reply(StatusCode::SERVICE_UNAVAILABLE, "A Moneta ainda não está pronta.");
        };
        let mut outgoing = self
            .client
            .request(method.clone(), format!("{}{path}", self.base()))
            .bearer_auth(key);
        for name in [header::CONTENT_TYPE, header::ACCEPT, header::ACCEPT_LANGUAGE] {
            if let Some(value) = request.headers().get(&name) {
                outgoing = outgoing.header(name, value);
            }
        }
        if method == Method::POST {
            // As rotas de escrita da Moneta conferem a origem; a página é dela, servida pela Ayla.
            outgoing = outgoing.header(header::ORIGIN, self.base()).body(request.into_body());
        }
        let response = match outgoing.send().await {
            Ok(response) => response,
            Err(_) => return reply(StatusCode::BAD_GATEWAY, "A Moneta não está respondendo."),
        };
        let mut builder = Response::builder().status(response.status());
        for (name, value) in response.headers() {
            if name == header::CONTENT_SECURITY_POLICY {
                if let Ok(policy) = value.to_str() {
                    builder = builder.header(name, embeddable_csp(policy));
                }
            } else if name == header::LOCATION {
                let location = value.to_str().unwrap_or("/");
                builder = builder.header(name, location.strip_prefix(&self.base()).unwrap_or(location));
            } else if [
                header::CONTENT_TYPE,
                header::CONTENT_DISPOSITION,
                header::CACHE_CONTROL,
                header::X_CONTENT_TYPE_OPTIONS,
                header::REFERRER_POLICY,
            ]
            .contains(name)
                || name.as_str() == "permissions-policy"
            {
                builder = builder.header(name, value);
            }
        }
        let html = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/html"));
        match response.bytes().await {
            Ok(body) if html => builder.body(with_theme(&String::from_utf8_lossy(&body)).into_bytes()).unwrap_or_default(),
            Ok(body) => builder.body(body.to_vec()).unwrap_or_default(),
            Err(_) => reply(StatusCode::BAD_GATEWAY, "A resposta da Moneta foi interrompida."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifesto_fixa_versao_e_contrato() {
        let m = manifest();
        assert_eq!(m.id, "moneta");
        assert_eq!(m.commit.len(), 40);
        assert!(m.commit.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(m.repository.starts_with("https://"));
        assert_eq!(m.contract, 1);
        // Porta própria: convive com a Moneta instalada separadamente (4173).
        assert_ne!(m.port, 4173);
        // A sobreposição troca só a pasta de dados e o reinício; o serviço é o do compose da Moneta.
        assert!(COMPOSE_OVERRIDE.contains(&format!("  {}:", m.service)));
        assert!(COMPOSE_OVERRIDE.contains("${AYLA_MONETA_DATA:?"));
        assert!(!COMPOSE_OVERRIDE.contains("build:"));
    }

    #[test]
    fn politica_permite_so_a_janela_da_ayla() {
        let original = "default-src 'none'; script-src 'self' https://cdn.pluggy.ai; frame-src https://*.pluggy.ai; base-uri 'none'; frame-ancestors 'none'";
        let csp = embeddable_csp(original);
        assert!(!csp.contains("frame-ancestors 'none'"));
        assert!(csp.contains("frame-ancestors 'self' tauri://localhost"));
        assert!(csp.contains("script-src 'self' https://cdn.pluggy.ai"));
        assert!(csp.contains("default-src 'none'"));
        assert_eq!(csp.matches("frame-ancestors").count(), 1);
    }

    #[test]
    fn paginas_do_orgao_seguem_o_tema_da_ayla() {
        let page = with_theme("<html><head><link rel=stylesheet href=/style.css></HEAD><body></body></html>");
        let link = page.find(THEME_PATH).unwrap();
        assert!(link > page.find("/style.css").unwrap(), "depois das folhas do órgão");
        assert!(link < page.to_ascii_lowercase().find("</head>").unwrap());
        let css = theme_css();
        assert!(css.find("--ayla-bg").unwrap() < css.find("--canvas: var(--ayla-bg)").unwrap());
        for token in ["--ayla-bg", "--ayla-font-main", "--ayla-text"] {
            assert!(ORGAN_THEME.contains(&format!("var({token})")), "{token}");
        }
        assert!(with_theme("sem head").starts_with("<link"));
    }

    #[test]
    fn financas_em_texto() {
        assert!(finance_question("Qual o valor da minha fatura em aberto?"));
        assert!(finance_question("quanto gastei este mês"));
        assert!(!finance_question("me conta uma piada"));
        assert_eq!(money(17999, "BRL"), "R$ 179,99");
        assert_eq!(money(-123456789, "BRL"), "-R$ 1.234.567,89");
        assert_eq!(money(5, "USD"), "USD 0,05");
        let resumo = serde_json::json!({
            "today":"2026-11-20","connected":true,
            "cards":[{"name":"Cartão","institution":"Banco","last4":"1234","currency":"BRL",
                "openInvoice":{"month":"2026-12","amount":17999,"closingDate":"2026-12-03","dueDate":"2026-12-10","estimated":true},
                "upcomingBills":[]}],
            "totals":[{"currency":"BRL","month":"2026-11","income":0,"spent":17999,"result":-17999,"openInvoices":17999,"netWorth":null}],
            "accounts":[{"name":"Conta corrente","institution":"Banco","balance":250000,"currency":"BRL"}],
            "recent":[{"date":"2026-11-18","description":"Mercado","amount":-5000,"currency":"BRL","category":"Alimentação","pending":false}]
        });
        let text = finance_text(&resumo);
        assert!(text.contains("- Cartão (Banco, final 1234): R$ 179,99, ainda em formação, fecha em 03/12/2026, vence em 10/12/2026."), "{text}");
        assert!(text.contains("Total das faturas em aberto (BRL): R$ 179,99."));
        assert!(text.contains("- Conta corrente (Banco): R$ 2.500,00."));
        assert!(text.contains("- 18/11/2026 Mercado -R$ 50,00 (Alimentação)"));
        assert!(text.contains("Mês 11/2026 (BRL): receitas R$ 0,00; despesas R$ 179,99; resultado -R$ 179,99."), "{text}");
    }

    #[test]
    fn caminhos_e_chaves() {
        assert_eq!(safe_path("/api/finance?x=1"), Some("/api/finance?x=1"));
        assert_eq!(safe_path("/"), Some("/"));
        assert_eq!(safe_path("/entrar?chave=abc"), None);
        assert_eq!(safe_path("//outro.site/"), None);
        assert_eq!(safe_path("api"), None);
        assert_eq!(safe_path("/a\nb"), None);
        let key = "A".repeat(43);
        assert_eq!(valid_key(&format!("{key}\n")), Some(key.clone()));
        assert_eq!(valid_key(&"A".repeat(42)), None);
        assert_eq!(valid_key(&format!("{}!", "A".repeat(42))), None);
    }

    /// Contêiner real, com dados descartáveis. Requer Docker e um repositório da Moneta já baixado
    /// em `$AYLA_ORGAN_TEST_ROOT/organs/moneta/source` (por `ayla --instalar-moneta` ou à mão).
    /// `AYLA_MONETA_PORT` evita a porta de uma Moneta em uso.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn contêiner_real_aparece_pela_camada() {
        let root = PathBuf::from(std::env::var("AYLA_ORGAN_TEST_ROOT").expect("AYLA_ORGAN_TEST_ROOT"));
        // Contêiner mesmo com a Moneta instalada neste computador (que nunca é tocada pelo teste).
        std::env::set_var("AYLA_MONETA_STANDALONE", "0");
        let organ = Organ::moneta(&root);
        fs::write(organ.dir.join("compose.ayla.yaml"), COMPOSE_OVERRIDE).unwrap();
        organ.prepare_data().unwrap();
        organ.run(organ.compose().args(["build", "--quiet", "moneta"]), "build").unwrap();
        fs::write(organ.marker(), &organ.manifest.commit).unwrap();
        assert!(organ.downloaded());

        let started = Instant::now();
        let health = organ.start().await.unwrap();
        println!("início: {:?}", started.elapsed());
        assert_eq!(health.api, 1);
        assert!(organ.container());
        assert!(organ.access_key().is_some(), "a chave nasce na pasta de dados da Ayla");

        let get = |path: &str| tauri::http::Request::builder().uri(format!("moneta://localhost{path}")).body(Vec::new()).unwrap();
        let page = organ.forward(get("/")).await;
        assert_eq!(page.status(), 200);
        let csp = page.headers().get("content-security-policy").unwrap().to_str().unwrap();
        assert!(csp.contains("frame-ancestors 'self' tauri://localhost") && !csp.contains("'none'; frame") && !csp.ends_with("frame-ancestors 'none'"));
        assert!(page.headers().get("x-frame-options").is_none());
        let html = String::from_utf8_lossy(page.body()).to_string();
        assert!(html.contains("<html") && html.contains(THEME_PATH), "a página recebe o tema da Ayla");
        let theme = organ.forward(get(THEME_PATH)).await;
        assert_eq!(theme.status(), 200);
        assert!(String::from_utf8_lossy(theme.body()).contains("--canvas: var(--ayla-bg)"));

        let resumo = organ.finance().await.unwrap();
        assert!(resumo["cards"].is_array() && resumo["totals"].is_array());
        let text = finance_text(&resumo);
        println!("{text}");
        // O contêiner conta os dias no fuso do computador.
        let local = Command::new("date").arg("+%Y-%m-%d").output().unwrap();
        assert_eq!(resumo["today"].as_str().unwrap(), String::from_utf8_lossy(&local.stdout).trim());
        assert!(text.starts_with("Dados da Moneta em "));

        let finance = organ.forward(get("/api/finance")).await;
        assert_eq!(finance.status(), 200, "a chave vai no cabeçalho, a página não a vê");
        let body: serde_json::Value = serde_json::from_slice(finance.body()).unwrap();
        assert!(body.get("accounts").is_some());
        assert_eq!(organ.forward(get("/entrar?chave=x")).await.status(), 403);

        let post = tauri::http::Request::builder().method("POST").uri("moneta://localhost/api/naoexiste")
            .header("content-type", "application/json").body(b"{}".to_vec()).unwrap();
        assert_ne!(organ.forward(post).await.status(), 403, "a origem da Moneta é aceita nas escritas");

        organ.stop(true);
        assert!(!organ.container());
        assert!(organ.health().await.is_none());
        let status = organ.status().await;
        assert!(status.installed && !status.running && status.icon.is_some());
        organ.remove().unwrap();
        assert!(!organ.downloaded());
        assert!(organ.data.join("acesso.key").is_file(), "remover não apaga os dados");
    }

    #[test]
    fn sem_download_nao_ha_instalacao() {
        let root = std::env::temp_dir().join(format!("ayla-organ-test-{}", std::process::id()));
        let organ = Organ::moneta(&root);
        assert!(!organ.downloaded());
        fs::create_dir_all(organ.source()).unwrap();
        fs::write(organ.source().join("compose.yaml"), "services: {}").unwrap();
        fs::write(organ.marker(), "0".repeat(40)).unwrap();
        assert!(!organ.downloaded(), "commit diferente do fixado não conta como instalado");
        fs::write(organ.marker(), &organ.manifest.commit).unwrap();
        assert!(organ.downloaded());
        let _ = fs::remove_dir_all(&root);
    }
}

// Comandos da interface. A mesma lógica atende `ayla --instalar-moneta`.

#[tauri::command]
pub async fn organ_status(organ: tauri::State<'_, Organ>) -> Result<Status, String> {
    Ok(organ.status().await)
}

#[tauri::command]
pub async fn organ_install(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || app.state::<Organ>().install(&|_| {}))
        .await
        .map_err(|_| "A instalação da Moneta foi interrompida".to_string())?
}

#[tauri::command]
pub async fn organ_start(organ: tauri::State<'_, Organ>) -> Result<Health, String> {
    organ.start().await
}

#[tauri::command]
pub async fn organ_remove(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || app.state::<Organ>().remove())
        .await
        .map_err(|_| "A remoção da Moneta foi interrompida".to_string())?
}

/// Versão separada: a mesma Moneta da Ayla numa janela própria.
#[tauri::command]
pub async fn organ_open_separate(app: tauri::AppHandle, organ: tauri::State<'_, Organ>) -> Result<(), String> {
    use tauri::Manager;
    organ.start().await?;
    if let Some(window) = app.get_webview_window(SEPARATE_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }
    let url = tauri::Url::parse(PAGE_URL).map_err(|_| "Endereço inválido")?;
    tauri::WebviewWindowBuilder::new(&app, SEPARATE_WINDOW, tauri::WebviewUrl::CustomProtocol(url))
        .title("Moneta")
        .inner_size(1280.0, 860.0)
        .min_inner_size(380.0, 560.0)
        .build()
        .map_err(|_| "Não foi possível abrir a janela da Moneta")?;
    Ok(())
}

/// Janelas autorizadas a usar o protocolo `moneta://`.
pub const SEPARATE_WINDOW: &str = "moneta";
pub const PAGE_URL: &str = "moneta://localhost/";
pub const SCHEME: &str = "moneta";

// Finanças para a conversa: a Ayla lê o resumo da Moneta (`GET /api/resumo`, contrato v1) e usa os
// números dela, sem refazer as regras da Moneta. Dados financeiros ficam neste computador.

/// Pedido que pede dados das finanças (fatura, saldo, gastos…). Busca simples por palavras.
pub fn finance_question(message: &str) -> bool {
    let text = message.to_lowercase();
    FINANCE_WORDS.iter().any(|word| text.contains(word))
}

/// Palavras que levam um pedido à Moneta; a interface usa a mesma lista (`organ_local_words`).
pub const FINANCE_WORDS: &[&str] = &[
        "fatura", "cartão", "cartao", "saldo", "gastei", "gasto", "despesa", "receita", "finança", "financa",
        "financeir", "dinheiro", "patrimônio", "patrimonio", "extrato", "transaç", "transac", "lançamento",
        "lancamento", "moneta", "quanto devo", "limite do", "conta corrente", "minhas contas", "salário", "salario",
        "investimento",
];

/// Palavras que fazem a conversa responder localmente com a Moneta (vazio sem a Moneta baixada).
#[tauri::command]
pub fn organ_local_words(organ: tauri::State<'_, Organ>) -> Vec<&'static str> {
    // Lembretes também são criados neste computador, em qualquer modo.
    let mut words = super::reminders::WORDS.to_vec();
    if organ.installed() {
        words.extend_from_slice(FINANCE_WORDS);
    }
    words
}

/// Valor em centavos no formato brasileiro: "R$ 1.234,56", "USD 12,34".
pub fn money(cents: i64, currency: &str) -> String {
    let negative = cents < 0;
    let cents = cents.unsigned_abs();
    let digits = (cents / 100).to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push('.');
        }
        grouped.push(c);
    }
    let symbol = if currency == "BRL" { "R$" } else { currency };
    format!("{}{symbol} {grouped},{:02}", if negative { "-" } else { "" }, cents % 100)
}

fn date_br(day: &str) -> String {
    let parts: Vec<&str> = day.split('-').collect();
    match parts.as_slice() {
        [y, m, d] => format!("{d}/{m}/{y}"),
        _ => day.to_string(),
    }
}

/// Texto compacto do resumo da Moneta: contexto do modelo local e resposta direta sem modelo.
pub fn finance_text(resumo: &serde_json::Value) -> String {
    use serde_json::Value;
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    let cents = |v: &Value| v.as_i64();
    let mut out = Vec::new();
    let today = s(&resumo["today"]);
    out.push(format!("Dados da Moneta em {}:", date_br(&today)));
    let cards = resumo["cards"].as_array().cloned().unwrap_or_default();
    let open: Vec<&Value> = cards.iter().filter(|c| c["openInvoice"].is_object()).collect();
    if open.is_empty() {
        out.push("Faturas em aberto: nenhuma identificada.".into());
    } else {
        out.push("Faturas em aberto (cartões de crédito):".into());
        for card in &open {
            let bill = &card["openInvoice"];
            let mut line = format!("- {} ({}", s(&card["name"]), s(&card["institution"]));
            if let Some(last4) = card["last4"].as_str() {
                line.push_str(&format!(", final {last4}"));
            }
            line.push_str(&format!("): {}", money(cents(&bill["amount"]).unwrap_or(0), &s(&card["currency"]))));
            if bill["estimated"].as_bool() == Some(true) {
                line.push_str(", ainda em formação");
            }
            if let Some(close) = bill["closingDate"].as_str() {
                line.push_str(&format!(", fecha em {}", date_br(close)));
            }
            if let Some(due) = bill["dueDate"].as_str() {
                line.push_str(&format!(", vence em {}", date_br(due)));
            }
            out.push(line + ".");
        }
    }
    for card in &cards {
        for bill in card["upcomingBills"].as_array().into_iter().flatten() {
            let (amount, paid) = (cents(&bill["amount"]).unwrap_or(0), cents(&bill["paid"]).unwrap_or(0));
            let currency = s(&bill["currency"]);
            out.push(format!(
                "- Fatura fechada de {} vence em {}: {}{}.",
                s(&card["name"]),
                date_br(&s(&bill["due"])),
                money(amount, &currency),
                if paid >= amount && amount > 0 { ", já paga".to_string() } else if paid > 0 { format!(", pago {}", money(paid, &currency)) } else { String::new() }
            ));
        }
    }
    for total in resumo["totals"].as_array().into_iter().flatten() {
        let currency = s(&total["currency"]);
        if let Some(open) = cents(&total["openInvoices"]) {
            out.push(format!("Total das faturas em aberto ({currency}): {}.", money(open, &currency)));
        }
        let month = s(&total["month"]);
        let mut line = format!("Mês {}/{} ({currency}):", &month.get(5..).unwrap_or(""), &month.get(..4).unwrap_or(""));
        for (key, label) in [("income", "receitas"), ("spent", "despesas"), ("result", "resultado"), ("netWorth", "patrimônio líquido")] {
            if let Some(value) = cents(&total[key]) {
                line.push_str(&format!(" {label} {};", money(value, &currency)));
            }
        }
        out.push(line.trim_end_matches(';').to_string() + ".");
    }
    let accounts = resumo["accounts"].as_array().cloned().unwrap_or_default();
    if !accounts.is_empty() {
        out.push("Contas:".into());
        for account in &accounts {
            let balance = cents(&account["balance"]).map(|b| money(b, &s(&account["currency"]))).unwrap_or_else(|| "saldo indisponível".into());
            out.push(format!("- {} ({}): {balance}.", s(&account["name"]), s(&account["institution"])));
        }
    }
    let recent = resumo["recent"].as_array().cloned().unwrap_or_default();
    if !recent.is_empty() {
        out.push("Lançamentos recentes:".into());
        for t in recent.iter().take(20) {
            let mut line = format!("- {} {} {}", date_br(&s(&t["date"])), s(&t["description"]), money(cents(&t["amount"]).unwrap_or(0), &s(&t["currency"])));
            if let Some(category) = t["category"].as_str() {
                line.push_str(&format!(" ({category})"));
            }
            if t["pending"].as_bool() == Some(true) {
                line.push_str(" pendente");
            }
            out.push(line);
        }
    }
    if resumo["connected"].as_bool() == Some(false) {
        out.push("A Moneta ainda não tem bancos conectados.".into());
    }
    out.join("\n")
}

impl Organ {
    /// Resumo financeiro atual. Sobe o contêiner se preciso; erros vêm em texto para a pessoa.
    pub async fn finance(&self) -> Result<serde_json::Value, String> {
        let health = self.start().await?;
        if health.locked {
            return Err("A Moneta está trancada. Abra a Moneta na Ayla para destrancar.".into());
        }
        let key = self.access_key().ok_or("A Moneta ainda não está pronta.")?;
        let response = self
            .client
            .get(format!("{}/api/resumo", self.base()))
            .bearer_auth(key)
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|_| "A Moneta não respondeu.")?;
        match response.status().as_u16() {
            200 => response.json().await.map_err(|_| "Resposta inválida da Moneta.".into()),
            404 => Err("Esta versão da Moneta não tem o resumo para a Ayla. Atualize a Moneta.".into()),
            423 => Err("A Moneta está trancada. Abra a Moneta na Ayla para destrancar.".into()),
            _ => Err("A Moneta não conseguiu montar o resumo.".into()),
        }
    }
}
