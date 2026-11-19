use std::fs::File;
use std::io::{self, Read};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tauri::Manager;

struct BackendState {
    child: Mutex<Option<Child>>,
    url: String,
    token: String,
}

impl BackendState {
    fn start() -> io::Result<Self> {
        // P0: desenvolvimento Linux. Distribuição será definida na próxima etapa.
        if !cfg!(target_os = "linux") || !cfg!(debug_assertions) {
            return Err(io::Error::other("Este P0 suporta desenvolvimento Linux; instaladores serão preparados depois."));
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent().and_then(Path::parent)
            .ok_or_else(|| io::Error::other("Diretório do projeto não encontrado"))?;
        let backend = root.join("backend");
        let python = std::env::var_os("AYLA_PYTHON")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| root.join(".venv/bin/python"));
        // Aleatoriedade do sistema operacional, sem dependência adicional.
        let mut random = [0u8; 32];
        File::open("/dev/urandom")?.read_exact(&mut random)?;
        let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let child = Command::new(python)
            .arg(backend.join("run.py"))
            .current_dir(&backend)
            .env("AYLA_TOKEN", &token)
            .env("AYLA_PORT", port.to_string())
            .stdin(Stdio::null())
            .spawn()?;
        Ok(Self { child: Mutex::new(Some(child)), url: format!("http://127.0.0.1:{port}"), token })
    }

    fn stop(&self) {
        if let Ok(mut guard) = self.child.lock() {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

impl Drop for BackendState {
    fn drop(&mut self) { self.stop(); }
}

#[tauri::command]
fn backend_connection(state: tauri::State<'_, BackendState>) -> Result<serde_json::Value, String> {
    let mut guard = state.child.lock().map_err(|_| "Falha ao acessar a Ayla")?;
    let child = guard.as_mut().ok_or("Ayla encerrada")?;
    if child.try_wait().map_err(|_| "Falha ao consultar a Ayla")?.is_some() {
        return Err("O serviço da Ayla não iniciou".into());
    }
    Ok(serde_json::json!({ "url": state.url, "token": state.token }))
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            app.manage(BackendState::start()?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![backend_connection])
        .build(tauri::generate_context!())
        .expect("Não foi possível iniciar a Ayla")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(state) = app.try_state::<BackendState>() { state.stop(); }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpStream;
    use std::time::{Duration, Instant};

    fn health(state: &BackendState, authenticated: bool) -> io::Result<String> {
        let address = state.url.trim_start_matches("http://");
        let mut socket = TcpStream::connect(address)?;
        socket.set_read_timeout(Some(Duration::from_secs(2)))?;
        let header = if authenticated { format!("X-Ayla-Token: {}\r\n", state.token) } else { String::new() };
        write!(socket, "GET /health HTTP/1.1\r\nHost: localhost\r\n{header}Connection: close\r\n\r\n")?;
        let mut response = String::new();
        socket.read_to_string(&mut response)?;
        Ok(response)
    }

    #[test]
    fn backend_lifecycle_without_ai() {
        // A execução deste teste deve usar banco temporário e IA desligada.
        let database = std::env::var("AYLA_DB_PATH").expect("Defina AYLA_DB_PATH temporário para o teste");
        assert!(!Path::new(&database).exists(), "O teste exige um banco novo e descartável");
        assert_eq!(std::env::var("AYLA_LOCAL_ENABLED").as_deref(), Ok("false"));
        assert_eq!(std::env::var("AYLA_CLOUD_ENABLED").as_deref(), Ok("false"));
        let start = Instant::now();
        let state = BackendState::start().expect("Iniciar backend Python");
        let mut ready = false;
        while start.elapsed() < Duration::from_secs(10) {
            if let Ok(response) = health(&state, true) {
                if response.starts_with("HTTP/1.1 200") && response.contains("\"status\":\"ok\"") {
                    assert!(response.contains("\"ai_configured\":false"));
                    ready = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(ready, "Backend precisa ficar pronto sem modelos ou rede externa");
        println!("Backend pronto sem IA em {} ms", start.elapsed().as_millis());
        let unauthorized = health(&state, false).expect("Consultar sem token");
        assert!(unauthorized.starts_with("HTTP/1.1 401"));
        assert!(!unauthorized.contains(&state.token));
        assert!(!Path::new(&database).exists(), "Inicialização e health não devem criar banco");
        state.stop();
        assert!(state.child.lock().unwrap().is_none());
        assert!(TcpStream::connect(state.url.trim_start_matches("http://")).is_err());
    }
}
