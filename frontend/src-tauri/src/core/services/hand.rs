//! Comandos do Hand (página do órgão e conversa): os serviços de `core::hand` com as pastas
//! liberadas, o histórico e os dados da Ayla fora da busca.
use super::*;
use crate::core::hand::{self, Busca, Captura, Lote, Passo, Tipo};

fn allowed(state: &Core) -> Result<Vec<PathBuf>, String> {
    Ok(state.prefs()?.hand_folders.unwrap_or_else(hand::default_folders))
}

fn history_path(state: &Core) -> PathBuf {
    state.root.join("hand").join("historico.json")
}

fn tool(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
}

/// Pastas liberadas, onde as capturas ficam e as ferramentas do sistema que o Hand usa.
#[tauri::command]
pub fn hand_estado(state: tauri::State<'_, Core>) -> Result<Value, String> {
    let prefs = state.prefs()?;
    let folders = prefs.hand_folders.clone().unwrap_or_else(hand::default_folders);
    let tools: serde_json::Map<String, Value> = ["grim", "slurp", "hyprctl", "plocate", "gio", "gtk-launch", "xdg-open", "wtype", "ydotool"]
        .iter().map(|t| (t.to_string(), json!(tool(t)))).collect();
    Ok(json!({"pastas": folders, "padrao": prefs.hand_folders.is_none(), "capturas": hand::screenshots_dir(), "home": hand::home(), "ferramentas": tools}))
}

/// Troca as pastas liberadas (`None` volta às padrão). Só pastas existentes da pasta pessoal,
/// fora de ocultas e dos dados da Ayla.
#[tauri::command]
pub fn hand_pastas(state: tauri::State<'_, Core>, pastas: Option<Vec<String>>) -> Result<(), String> {
    let home = hand::home();
    let folders = match pastas {
        None => None,
        Some(list) => {
            let mut out: Vec<PathBuf> = Vec::new();
            for folder in list {
                let path = PathBuf::from(&folder);
                if !path.is_dir() { return Err(format!("{folder} não é uma pasta")); }
                if path == home || !hand::inside(&path, std::slice::from_ref(&home)) { return Err(format!("{folder}: escolha uma pasta dentro da pasta pessoal (não ela inteira nem uma oculta)")); }
                if path.starts_with(&state.root) || state.root.starts_with(&path) { return Err("Os dados da Ayla ficam fora do Hand".into()); }
                if !out.contains(&path) { out.push(path); }
            }
            Some(out)
        }
    };
    let store = state.preferences.lock().map_err(|_| "Configuração ocupada")?;
    let mut prefs = store.load().map_err(|_| "Não foi possível ler a configuração")?;
    prefs.hand_folders = folders;
    store.save(&prefs).map_err(|_| "Não foi possível salvar as pastas".into())
}

/// Captura de tela (inteira, região ou janela), salva em Imagens/Capturas de tela.
/// `None` quando a pessoa cancela a escolha.
#[tauri::command]
pub async fn hand_capturar(tipo: Captura, espera: Option<u64>) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || hand::capture(tipo, &hand::screenshots_dir(), espera.unwrap_or(0)))
        .await.map_err(|_| "A captura parou".to_string())?
        .map(|path| path.map(|p| p.to_string_lossy().into_owned()))
}

/// Busca pelo nome na pasta pessoal (sem ocultas e sem os dados da Ayla).
#[tauri::command]
pub async fn hand_buscar(state: tauri::State<'_, Core>, busca: String, tipo: Option<Tipo>, dias: Option<u32>) -> Result<Vec<hand::Achado>, String> {
    let exclude = vec![state.root.clone()];
    tauri::async_runtime::spawn_blocking(move || {
        let fresh = hand::default_folders();
        hand::search(&Busca { query: &busca, tipo, dias, limit: 100 }, &hand::home(), &exclude, &fresh)
    }).await.map_err(|_| "A busca parou".into())
}

/// Plano para arrumar uma pasta liberada: por tipo ou por data. Nada muda até Aplicar.
#[tauri::command]
pub fn hand_planejar(state: tauri::State<'_, Core>, pasta: String, modo: String) -> Result<Vec<Passo>, String> {
    let folder = PathBuf::from(&pasta);
    let folders = allowed(&state)?;
    if !folder.is_dir() || !(folders.contains(&folder) || hand::inside(&folder, &folders)) { return Err("Essa pasta não está liberada para o Hand".into()); }
    let plan = match modo.as_str() { "data" => hand::plan_by_date(&folder), _ => hand::plan_by_type(&folder) };
    if plan.is_empty() { return Err("Nada para arrumar: não há arquivos soltos com tipo conhecido".into()); }
    Ok(plan)
}

static HAND_GATE: Mutex<()> = Mutex::new(());

/// Executa um plano mostrado à pessoa (só depois do clique em Aplicar) e guarda no histórico.
#[tauri::command]
pub async fn hand_aplicar(state: tauri::State<'_, Core>, plano: Vec<Passo>, titulo: String, origem: Option<String>) -> Result<hand::Resultado, String> {
    let folders = allowed(&state)?;
    let history = history_path(&state);
    tauri::async_runtime::spawn_blocking(move || {
        let _gate = HAND_GATE.lock().map_err(|_| "O Hand está ocupado")?;
        let mut result = hand::apply(&plano, &folders, &titulo)?;
        result.lote.origem = origem;
        if !result.lote.feitos.is_empty() {
            let mut lotes = hand::history_load(&history);
            lotes.push(result.lote.clone());
            hand::history_save(&history, &lotes)?;
        }
        Ok(result)
    }).await.map_err(|_| "O Hand parou".to_string())?
}

#[tauri::command]
pub fn hand_historico(state: tauri::State<'_, Core>) -> Vec<Lote> {
    let mut lotes = hand::history_load(&history_path(&state));
    lotes.reverse();
    lotes
}

/// Desfaz um lote do histórico (uma vez).
#[tauri::command]
pub async fn hand_desfazer(state: tauri::State<'_, Core>, id: String) -> Result<Value, String> {
    let history = history_path(&state);
    tauri::async_runtime::spawn_blocking(move || {
        let _gate = HAND_GATE.lock().map_err(|_| "O Hand está ocupado")?;
        let mut lotes = hand::history_load(&history);
        let lote = lotes.iter_mut().find(|l| l.id == id).ok_or("Esse lote não está mais no histórico")?;
        if lote.desfeito { return Err("Esse lote já foi desfeito".into()); }
        let (voltaram, problemas) = hand::undo(lote);
        lote.desfeito = true;
        hand::history_save(&history, &lotes)?;
        Ok(json!({"voltaram": voltaram, "problemas": problemas}))
    }).await.map_err(|_| "O Hand parou".to_string())?
}

#[tauri::command]
pub async fn hand_programas() -> Result<Vec<hand::Programa>, String> {
    tauri::async_runtime::spawn_blocking(hand::programs).await.map_err(|_| "Falha ao listar os programas".into())
}

#[tauri::command]
pub fn hand_abrir_programa(id: String) -> Result<(), String> {
    hand::launch(&id)
}

/// Abre um arquivo ou pasta da pasta pessoal (ou mostra o arquivo na pasta dele).
#[tauri::command]
pub fn hand_abrir(state: tauri::State<'_, Core>, caminho: String, mostrar: Option<bool>) -> Result<(), String> {
    let path = PathBuf::from(&caminho);
    if !hand::inside(&path, &[hand::home()]) || path.starts_with(&state.root) { return Err("Fora da pasta pessoal".into()); }
    hand::open_path(&path, mostrar.unwrap_or(false))
}

/// Roda um comando no shell, na pasta pessoal (até 60 s). A interface mostra o comando e só
/// chama depois da confirmação da pessoa.
#[tauri::command]
pub async fn hand_comando(comando: String) -> Result<hand::Saida, String> {
    hand::run(&comando, &hand::home(), std::time::Duration::from_secs(60)).await
}

fn random_id() -> String {
    let mut bytes = [0u8; 8];
    let _ = getrandom::fill(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Pedido ao Hand na conversa, sem modelo e sempre neste computador. `None`: não era pedido ao
/// Hand (ou o programa pedido não existe), e a conversa segue normal.
pub(super) async fn hand_conversa(state: &Core, message: &str, events: &Channel<Value>, extras: &std::sync::Mutex<Value>) -> Result<Option<String>, String> {
    use hand::Pedido;
    let Some(pedido) = hand::pedido(message) else { return Ok(None) };
    let home = hand::home();
    let short = |p: &std::path::Path| { let text = p.to_string_lossy().into_owned(); match text.strip_prefix(&*home.to_string_lossy()) { Some(rest) => format!("~{rest}"), None => text } };
    let reply = match pedido {
        Pedido::Captura(tipo) => {
            let _ = events.send(json!({"type":"status","text":"Capturando a tela…"}));
            let shot = tauri::async_runtime::spawn_blocking(move || hand::capture(tipo, &hand::screenshots_dir(), 0)).await.map_err(|_| "A captura parou")??;
            match shot {
                None => "Captura cancelada.".to_string(),
                Some(path) => {
                    send_kept(events, extras, json!({"type":"hand","item":{"home": home, "tipo":"captura","caminho": path}}));
                    format!("Salvei a captura em `{}`.", short(&path))
                }
            }
        }
        Pedido::Busca { consulta, tipo, dias } => {
            let _ = events.send(json!({"type":"status","text":"Procurando nos seus arquivos…"}));
            let exclude = vec![state.root.clone()];
            let query = consulta.clone();
            let found = tauri::async_runtime::spawn_blocking(move || hand::search(&Busca { query: &query, tipo, dias, limit: 30 }, &hand::home(), &exclude, &hand::default_folders()))
                .await.map_err(|_| "A busca parou")?;
            let what = if consulta.is_empty() { "isso".to_string() } else { format!("“{consulta}”") };
            if found.is_empty() { format!("Não achei {what} na sua pasta pessoal (sem contar pastas ocultas).") } else {
                send_kept(events, extras, json!({"type":"hand","item":{"home": home, "tipo":"busca","consulta": consulta, "achados": found}}));
                let n = found.len();
                format!("Achei {} {} para {what}{}. Os mais prováveis estão abaixo.", n, if n == 1 { "resultado" } else { "resultados" }, if n >= 30 { " (mostro os 30 primeiros)" } else { "" })
            }
        }
        Pedido::Organizar { pasta, por_data } => {
            let folders = allowed(state)?;
            match hand::pasta_pelo_nome(&pasta, &folders) {
                None => format!("A pasta “{pasta}” não está entre as liberadas para o Hand. Libere-a em Hand › Pastas liberadas e peça de novo."),
                Some(folder) => {
                    let plan = if por_data { hand::plan_by_date(&folder) } else { hand::plan_by_type(&folder) };
                    if plan.is_empty() { format!("`{}` já está arrumada: não há arquivos soltos com tipo conhecido.", short(&folder)) } else {
                        let title = format!("{} por {}", folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), if por_data { "data" } else { "tipo" });
                        let moves = plan.iter().filter(|p| !matches!(p, Passo::Pasta { .. })).count();
                        send_kept(events, extras, json!({"type":"hand","item":{"home": home, "tipo":"plano","id": random_id(), "titulo": title, "passos": plan}}));
                        format!("Montei o plano para arrumar `{}`: {moves} {} para mover. Nada muda até você clicar em **Aplicar**; depois dá para desfazer no Hand › Histórico.", short(&folder), if moves == 1 { "arquivo" } else { "arquivos" })
                    }
                }
            }
        }
        Pedido::Programa(nome) => {
            let programas = tauri::async_runtime::spawn_blocking(hand::programs).await.map_err(|_| "Falha ao listar os programas")?;
            let Some(programa) = hand::programa_pelo_nome(&nome, &programas) else { return Ok(None) };
            hand::launch(&programa.id)?;
            send_kept(events, extras, json!({"type":"hand","item":{"home": home, "tipo":"programa","nome": programa.name, "id": programa.id}}));
            format!("Abri o **{}**.", programa.name)
        }
        Pedido::Comando(comando) => {
            send_kept(events, extras, json!({"type":"hand","item":{"home": home, "tipo":"comando","id": random_id(), "comando": comando}}));
            "Confira o comando abaixo e clique em **Rodar** para ele rodar no seu computador (na pasta pessoal, por até 60 s).".to_string()
        }
    };
    let _ = events.send(json!({"type":"routing","text":"Hand · feito neste computador"}));
    Ok(Some(reply))
}
