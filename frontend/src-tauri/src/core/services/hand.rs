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

/// Busca pelo nome na pasta pessoal ou em todo o computador (`tudo`), com ou sem ocultos, por
/// tipos e data. Os dados da Ayla nunca entram.
#[tauri::command]
pub async fn hand_buscar(state: tauri::State<'_, Core>, busca: String, tipos: Option<Vec<Tipo>>, dias: Option<u32>, ocultos: Option<bool>, tudo: Option<bool>) -> Result<Vec<hand::Achado>, String> {
    let exclude = vec![state.root.clone()];
    tauri::async_runtime::spawn_blocking(move || {
        let fresh = hand::default_folders();
        let tipos = tipos.unwrap_or_default();
        let root = if tudo.unwrap_or(false) { PathBuf::from("/") } else { hand::home() };
        hand::search(&Busca { query: &busca, tipos: &tipos, ocultos: ocultos.unwrap_or(false), dias, limit: 150 }, &root, &exclude, &fresh)
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

/// Arquivos guardados na Iris (`files/<id>/<nome>` nos dados da Ayla) para os passos do Hand.
struct IrisDaAyla<'a>(&'a Core);

impl hand::IrisOrigem for IrisDaAyla<'_> {
    fn tirar(&self, id: &str) -> Result<(PathBuf, hand::IrisRegistro), String> {
        let file = self.0.store.lock().map_err(|_| "Dados ocupados")?.file(id).map_err(|_| "Falha ao ler a Iris")?.ok_or("Não está mais na Iris")?;
        if !matches!(file.kind, Kind::Image | Kind::Gif | Kind::Video | Kind::Pdf) { return Err("Só imagens, vídeos e PDFs da Iris".into()); }
        let dir = self.0.root.join("files").join(&file.id);
        let path = dir.join(&file.name);
        if file.path != format!("files/{}/{}", file.id, file.name) || !path.is_file() { return Err("O arquivo não está mais na Iris".into()); }
        let creditos = fs::read_to_string(dir.join("creditos.json")).ok();
        Ok((path, hand::IrisRegistro { id: file.id, name: file.name, kind: file.kind.as_str().to_string(), bytes: file.bytes, created_at: file.created_at,
            conversation_id: file.conversation_id, project_id: file.project_id, message_id: file.message_id, creditos }))
    }

    fn esquecer(&self, registro: &hand::IrisRegistro) -> Result<(), String> {
        self.0.store.lock().map_err(|_| "Dados ocupados")?.delete_file(&registro.id).map_err(|_| "Não foi possível tirar da Iris")?;
        let dir = self.0.root.join("files").join(&registro.id);
        let _ = fs::remove_file(dir.join("creditos.json"));
        let _ = fs::remove_dir(&dir);
        Ok(())
    }

    fn devolver(&self, de: &std::path::Path, registro: &hand::IrisRegistro) -> Result<(), String> {
        let kind = Kind::parse(&registro.kind).ok_or("Tipo inválido")?;
        let name = files::safe_name(&registro.name);
        if registro.id.is_empty() || registro.id.contains(['/', '.']) || name != registro.name { return Err("Registro inválido".into()); }
        let dir = self.0.root.join("files").join(&registro.id);
        let dest = dir.join(&name);
        if dest.exists() { return Err("Já existe na Iris".into()); }
        fs::create_dir_all(&dir).map_err(|_| "Não foi possível devolver à Iris")?;
        hand::move_path(de, &dest).map_err(|e| e.to_string())?;
        let back = || { let _ = hand::move_path(&dest, de); let _ = fs::remove_dir_all(&dir); };
        let sha = match models::hash(&dest) { Ok(sha) => sha, Err(_) => { back(); return Err("Não foi possível ler o arquivo".into()); } };
        let info = FileInfo { id: registro.id.clone(), name: name.clone(), kind, bytes: registro.bytes, created_at: registro.created_at, conversation_id: registro.conversation_id.clone(),
            project_id: registro.project_id.clone(), message_id: registro.message_id, path: format!("files/{}/{name}", registro.id), extracted: None };
        let inserted = self.0.store.lock().map_err(|_| "Dados ocupados".to_string()).and_then(|db| db.insert_file(&info, &sha, true).map_err(|_| "Não foi possível registrar na Iris".to_string()));
        if let Err(error) = inserted { back(); return Err(error); }
        if let Some(creditos) = &registro.creditos { let _ = fs::write(dir.join("creditos.json"), creditos); }
        Ok(())
    }
}

static HAND_GATE: Mutex<()> = Mutex::new(());

/// Executa um plano mostrado à pessoa (só depois do clique em Aplicar) e guarda no histórico.
#[tauri::command]
pub async fn hand_aplicar(app: tauri::AppHandle, state: tauri::State<'_, Core>, plano: Vec<Passo>, titulo: String, origem: Option<String>) -> Result<hand::Resultado, String> {
    use tauri::Manager;
    let folders = allowed(&state)?;
    let history = history_path(&state);
    // O registro para desfazer é sempre o da Ayla, nunca o que veio da página.
    let plano: Vec<Passo> = plano.into_iter().map(|p| match p { Passo::Iris { arquivo, nome, para, .. } => Passo::Iris { arquivo, nome, para, registro: None }, p => p }).collect();
    tauri::async_runtime::spawn_blocking(move || {
        let _gate = HAND_GATE.lock().map_err(|_| "O Hand está ocupado")?;
        let core = app.state::<Core>();
        let mut result = hand::apply(&plano, &folders, &titulo, Some(&IrisDaAyla(&core)))?;
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
pub async fn hand_desfazer(app: tauri::AppHandle, state: tauri::State<'_, Core>, id: String) -> Result<Value, String> {
    use tauri::Manager;
    let history = history_path(&state);
    tauri::async_runtime::spawn_blocking(move || {
        let _gate = HAND_GATE.lock().map_err(|_| "O Hand está ocupado")?;
        let core = app.state::<Core>();
        let mut lotes = hand::history_load(&history);
        let lote = lotes.iter_mut().find(|l| l.id == id).ok_or("Esse lote não está mais no histórico")?;
        if lote.desfeito { return Err("Esse lote já foi desfeito".into()); }
        let (voltaram, problemas) = hand::undo(lote, Some(&IrisDaAyla(&core)));
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

/// Abre um arquivo ou pasta achado pela busca (ou mostra o arquivo na pasta dele).
#[tauri::command]
pub fn hand_abrir(state: tauri::State<'_, Core>, caminho: String, mostrar: Option<bool>) -> Result<(), String> {
    let path = PathBuf::from(&caminho);
    // Qualquer arquivo que a busca mostra (inclusive ocultos e fora da pasta pessoal), menos os
    // dados da Ayla e as pastas virtuais do sistema.
    let ok = path.is_absolute() && !path.components().any(|c| matches!(c, std::path::Component::ParentDir))
        && !path.starts_with(&state.root) && !hand::VIRTUAIS.iter().any(|v| path.starts_with(v));
    if !ok { return Err("Esse caminho não pode ser aberto pelo Hand".into()); }
    hand::open_path(&path, mostrar.unwrap_or(false))
}

/// Roda um comando no shell, na pasta pessoal (até 60 s). A interface mostra o comando e só
/// chama depois da confirmação da pessoa.
#[tauri::command]
pub async fn hand_comando(comando: String) -> Result<hand::Saida, String> {
    hand::run(&comando, &hand::home(), std::time::Duration::from_secs(60)).await
}

/// Imagem que os cartões podem mostrar e usar: na pasta pessoal ou numa pasta liberada, sem `..`.
fn visivel(state: &Core, imagem: &std::path::Path) -> bool {
    imagem.is_absolute() && !imagem.components().any(|c| c == std::path::Component::ParentDir) && hand::tipo_de(imagem) == Some(Tipo::Imagem)
        && (imagem.starts_with(hand::home()) || allowed(state).is_ok_and(|pastas| pastas.iter().any(|p| imagem.starts_with(p))))
}

/// Põe a imagem como papel de parede (cartão da conversa: "Definir" e "Voltar ao anterior").
#[tauri::command]
pub async fn hand_papel_de_parede(state: tauri::State<'_, Core>, caminho: String) -> Result<Value, String> {
    let imagem = PathBuf::from(&caminho);
    if !visivel(&state, &imagem) { return Err("Só imagens da sua pasta pessoal ou das pastas liberadas".into()); }
    tauri::async_runtime::spawn_blocking(move || {
        let fundo = hand::fundo_do_sistema().ok_or("Não sei trocar o papel de parede neste sistema")?;
        let anterior = hand::fundo_atual(&fundo);
        hand::definir_fundo(&fundo, &imagem)?;
        Ok(json!({"anterior": anterior, "programa": fundo.nome()}))
    }).await.map_err(|_| "O Hand parou")?
}

/// Miniatura de uma imagem da pasta pessoal (para os cartões do Hand), em JPEG.
#[tauri::command]
pub async fn hand_miniatura(state: tauri::State<'_, Core>, caminho: String) -> Result<tauri::ipc::Response, String> {
    let imagem = PathBuf::from(&caminho);
    if !visivel(&state, &imagem) { return Err("Só imagens da sua pasta pessoal ou das pastas liberadas".into()); }
    let cache = super::thumbnails(&state);
    tauri::async_runtime::spawn_blocking(move || {
        let thumb = crate::core::media::thumbnail(&cache, &imagem, false).ok_or("Sem miniatura")?;
        fs::read(thumb).map(tauri::ipc::Response::new).map_err(|_| "Sem miniatura".to_string())
    }).await.map_err(|_| "O Hand parou")?
}

fn random_id() -> String {
    let mut bytes = [0u8; 8];
    let _ = getrandom::fill(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// O que as respostas anteriores do Hand nesta conversa deixaram: os arquivos movidos, achados
/// ou capturados (para "um deles", "esses") e a pergunta que ficou esperando resposta.
#[derive(Default, Debug)]
pub(super) struct Contexto {
    pub arquivos: Vec<PathBuf>,
    pub pergunta: Option<String>,
    /// Tipo do último cartão ("papel_de_parede", "plano"…).
    pub tipo: String,
}

impl Contexto {
    /// `anteriores`: extras das respostas, da mais recente para a mais antiga. Vale o último
    /// cartão do Hand.
    pub fn de(anteriores: &[Value]) -> Self {
        let Some(card) = anteriores.iter().map(|e| &e["hand"]).find(|c| c.is_object()) else { return Self::default() };
        let caminho = |v: &Value| v.as_str().map(PathBuf::from);
        let mut arquivos: Vec<PathBuf> = match card["tipo"].as_str() {
            Some("plano") => serde_json::from_value::<Vec<Passo>>(card["passos"].clone()).unwrap_or_default().into_iter().filter_map(|p| match p {
                // Aplicado: está no destino; senão, ainda na origem.
                Passo::Mover { de, para } => Some(if std::path::Path::new(&para).exists() { PathBuf::from(para) } else { PathBuf::from(de) }),
                Passo::Iris { para, .. } => Some(PathBuf::from(para)),
                _ => None,
            }).collect(),
            Some("busca") => card["achados"].as_array().into_iter().flatten().filter_map(|a| caminho(&a["path"])).collect(),
            Some("captura") => caminho(&card["caminho"]).into_iter().collect(),
            Some("papel_de_parede") => card["opcoes"].as_array().into_iter().flatten().filter_map(caminho).collect(),
            _ => Vec::new(),
        };
        arquivos.retain(|p| p.exists());
        arquivos.truncate(30);
        let pergunta = (card["tipo"] == "pergunta").then(|| card["objetivo"].as_str().map(str::to_string)).flatten();
        Self { arquivos, pergunta, tipo: card["tipo"].as_str().unwrap_or_default().to_string() }
    }
    fn texto(&self) -> String {
        if self.arquivos.is_empty() { return String::new(); }
        format!("arquivos: {}", self.arquivos.iter().map(|p| format!("\"{}\"", p.display())).collect::<Vec<_>>().join(", "))
    }
}

/// Resposta curta à pergunta "pela Ayla ou pelo navegador?" ("pela ayla", "foi no navegador").
pub(super) fn responde_origem(message: &str) -> bool {
    hand::pedido(message).is_none() && message.split_whitespace().count() <= 6 && !hand::origem_ambigua(&format!("baixei {message}"))
}

/// Resposta à pergunta do Hand logo depois dela: fica neste computador (o Automático não classifica).
/// Também "usa a noite" logo depois do cartão do papel de parede.
pub(crate) fn segue_hand(state: &Core, conversation: Option<&str>, edit_of: Option<i64>, regenerate: Option<i64>, message: &str) -> bool {
    let Some(conversation) = conversation.filter(|_| message.split_whitespace().count() <= 6) else { return false };
    let Ok(db) = state.store.lock() else { return false };
    let Ok((placement, _)) = db.placement(conversation, edit_of, regenerate) else { return false };
    let Some(ultimo) = db.recent_extras(conversation, placement.parent, 1).ok().and_then(|l| l.into_iter().next()) else { return false };
    drop(db);
    let contexto = Contexto::de(&[ultimo]);
    (contexto.pergunta.is_some() && responde_origem(message))
        || (contexto.tipo == "papel_de_parede" && hand::pista_de_imagem(message, &contexto.arquivos).is_some())
}

/// Pedido ao Hand na conversa, sem modelo e sempre neste computador. `None`: não era pedido ao
/// Hand (ou o programa pedido não existe), e a conversa segue normal.
/// `forcar`: a pessoa marcou "Usar o computador" ou o entendimento reconheceu um pedido ao
/// computador; o que não for um pedido claro vira tarefa (plano para aprovar).
/// `anteriores`: extras das respostas anteriores desta conversa (mais recente primeiro).
pub(super) async fn hand_conversa(state: &Core, message: &str, forcar: bool, anteriores: &[Value], events: &Channel<Value>, extras: &std::sync::Mutex<Value>) -> Result<Option<String>, String> {
    let status = |text: &str| { let _ = events.send(json!({"type":"status","text":text})); };
    let contexto = Contexto::de(anteriores);
    // Resposta curta à pergunta do Hand ("pela Ayla", "pelo navegador"): junta com o pedido.
    let (message, forcar) = match &contexto.pergunta {
        Some(objetivo) if responde_origem(message) => (format!("{}, {objetivo}", message.trim().trim_end_matches(['.', '!'])), true),
        _ => (message.to_string(), forcar),
    };
    // "usa a noite" logo depois do cartão do papel de parede: escolhe entre as mesmas opções.
    let claro = hand::pedido(&message).or_else(|| (contexto.tipo == "papel_de_parede" && message.split_whitespace().count() <= 6
        && hand::pista_de_imagem(&message, &contexto.arquivos).is_some()).then(|| hand::Pedido::PapelDeParede(message.clone())));
    let resolvido = match claro { Some(pedido) => resolver(state, pedido, &contexto, &status).await?, None => None };
    let resolvido = match resolvido {
        Some(found) => Some(found),
        None if forcar => resolver(state, hand::Pedido::Tarefa(message.trim().to_string()), &contexto, &status).await?,
        None => None,
    };
    let Some((card, reply)) = resolvido else { return Ok(None) };
    if let Some(card) = card { send_kept(events, extras, json!({"type":"hand","item":card})); }
    let _ = events.send(json!({"type":"routing","text":"Hand · feito neste computador"}));
    Ok(Some(reply))
}

/// Faz o pedido (o mesmo na conversa e em Hand › Pedir uma tarefa): o cartão a mostrar, se houver,
/// e o texto da resposta. `None`: não dá para atender (programa que não existe).
pub(super) async fn resolver(state: &Core, pedido: hand::Pedido, contexto: &Contexto, status: &(dyn Fn(&str) + Sync)) -> Result<Option<(Option<Value>, String)>, String> {
    use hand::Pedido;
    let home = hand::home();
    let mut card: Option<Value> = None;
    let short = |p: &std::path::Path| { let text = p.to_string_lossy().into_owned(); match text.strip_prefix(&*home.to_string_lossy()) { Some(rest) => format!("~{rest}"), None => text } };
    let reply = match pedido {
        Pedido::Captura(tipo) => {
            status("Capturando a tela…");
            let shot = tauri::async_runtime::spawn_blocking(move || hand::capture(tipo, &hand::screenshots_dir(), 0)).await.map_err(|_| "A captura parou")??;
            match shot {
                None => "Captura cancelada.".to_string(),
                Some(path) => {
                    card = Some(json!({"home": home, "tipo":"captura","caminho": path}));
                    format!("Salvei a captura em `{}`.", short(&path))
                }
            }
        }
        Pedido::Busca { consulta, tipo, dias } => {
            status("Procurando nos seus arquivos…");
            let exclude = vec![state.root.clone()];
            let query = consulta.clone();
            let found = tauri::async_runtime::spawn_blocking(move || hand::search(&Busca { query: &query, tipos: tipo.as_slice(), ocultos: false, dias, limit: 30 }, &hand::home(), &exclude, &hand::default_folders()))
                .await.map_err(|_| "A busca parou")?;
            let what = if consulta.is_empty() { "isso".to_string() } else { format!("“{consulta}”") };
            if found.is_empty() { format!("Não achei {what} na sua pasta pessoal (sem contar pastas ocultas).") } else {
                card = Some(json!({"home": home, "tipo":"busca","consulta": consulta, "achados": found}));
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
                        card = Some(json!({"home": home, "tipo":"plano","id": random_id(), "titulo": title, "passos": plan}));
                        format!("Montei o plano para arrumar `{}`: {moves} {} para mover. Nada muda até você clicar em **Aplicar**; depois dá para desfazer no Hand › Histórico.", short(&folder), if moves == 1 { "arquivo" } else { "arquivos" })
                    }
                }
            }
        }
        Pedido::Programa(nome) => {
            let programas = tauri::async_runtime::spawn_blocking(hand::programs).await.map_err(|_| "Falha ao listar os programas")?;
            let Some(programa) = hand::programa_pelo_nome(&nome, &programas) else { return Ok(None) };
            hand::launch(&programa.id)?;
            card = Some(json!({"home": home, "tipo":"programa","nome": programa.name, "id": programa.id}));
            format!("Abri o **{}**.", programa.name)
        }
        Pedido::DaIris { pasta, quantos, tipo } => {
            let folders = allowed(state)?;
            match hand::pasta_destino(&pasta, &folders) {
                None => format!("Não achei a pasta “{pasta}” nas pastas liberadas para o Hand ({}). Libere-a em Hand › Pastas liberadas ou diga o nome dela como aparece no computador.",
                    folders.iter().map(|f| format!("`{}`", short(f))).collect::<Vec<_>>().join(", ")),
                Some(folder) => {
                    let kinds: &[Kind] = match tipo { Tipo::Video => &[Kind::Video], Tipo::Pdf => &[Kind::Pdf], _ => &[Kind::Image, Kind::Gif] };
                    let media = state.store.lock().map_err(|_| "Dados ocupados")?.media_files().map_err(|_| "Falha ao ler a Iris")?;
                    let guardados: Vec<FileInfo> = media.into_iter().map(|(file, ..)| file).filter(|f| kinds.contains(&f.kind) && f.path == format!("files/{}/{}", f.id, f.name)).collect();
                    let ids = hand::recentes(&guardados.iter().map(|f| (f.id.clone(), f.created_at)).collect::<Vec<_>>(), quantos);
                    let escolhidos: Vec<&FileInfo> = ids.iter().filter_map(|id| guardados.iter().find(|f| &f.id == id)).collect();
                    let (um, varios) = match tipo { Tipo::Video => ("vídeo", "vídeos"), Tipo::Pdf => ("PDF", "PDFs"), _ => ("imagem", "imagens") };
                    if escolhidos.is_empty() { format!("Não há {varios} guardadas na Iris.") } else {
                        let plan = hand::plan_from_iris(&escolhidos.iter().map(|f| (f.id.clone(), f.name.clone())).collect::<Vec<_>>(), &folder);
                        let n = escolhidos.len();
                        let desde = escolhidos.last().map(|f| chrono::DateTime::from_timestamp(f.created_at, 0).map(|d| d.with_timezone(&chrono::Local).format("%d/%m às %H:%M").to_string()).unwrap_or_default()).unwrap_or_default();
                        let title = format!("Iris → {}", folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                        card = Some(json!({"home": home, "tipo":"plano","id": random_id(), "titulo": title, "passos": plan}));
                        format!("Montei o plano para mover {} {} da Iris para `{}` ({}salvas desde {desde}). Elas saem da Iris. Nada muda até você clicar em **Aplicar**; o Hand › Histórico desfaz e devolve tudo para a Iris.",
                            if quantos.is_some() { format!("as {n} últimas") } else { format!("{n}") }, if n == 1 { um } else { varios }, short(&folder), if quantos.is_some() { "" } else { "a última leva, " })
                    }
                }
            }
        }
        Pedido::Tarefa(objetivo) if hand::origem_ambigua(&objetivo) => {
            card = Some(json!({"home": home, "tipo":"pergunta","objetivo": objetivo}));
            "Antes de mexer nos arquivos: você baixou **pela Ayla** (eles ficam na Iris) ou **pelo navegador** (ficam em Downloads)? Pode responder só isso.".to_string()
        }
        Pedido::PapelDeParede(texto) => {
            let Some(fundo) = tauri::async_runtime::spawn_blocking(hand::fundo_do_sistema).await.map_err(|_| "O Hand parou")? else {
                return Ok(Some((None, "Não sei trocar o papel de parede neste sistema: não achei swww, hyprpaper, o Quickshell (illogical-impulse), o Plasma nem o GNOME.".to_string())));
            };
            // As opções: a pasta citada, senão o que a conversa acabou de mover ou achar.
            let folders = allowed(state)?;
            let citada = hand::pasta_citada(&texto);
            let pasta = citada.as_deref().and_then(|nome| hand::pasta_destino(nome, &folders).or_else(|| hand::pasta_destino(nome, &hand::default_folders())));
            let opcoes: Vec<PathBuf> = match &pasta {
                Some(dir) => { let dir = dir.clone(); tauri::async_runtime::spawn_blocking(move || hand::imagens_da_pasta(&dir, 24)).await.unwrap_or_default() }
                None => contexto.arquivos.iter().filter(|p| hand::tipo_de(p) == Some(Tipo::Imagem)).take(24).cloned().collect(),
            };
            if opcoes.is_empty() {
                return Ok(Some((None, match (citada, pasta) {
                    (Some(nome), None) => format!("Não achei a pasta “{nome}”. Diga o nome dela como aparece no computador."),
                    (_, Some(dir)) => format!("Não há imagens em `{}`.", short(&dir)),
                    _ => "Qual imagem? Diga a pasta (“a mais recente da pasta Wallpapers”) ou peça logo depois de mover ou achar as imagens.".to_string(),
                })));
            }
            let escolhida = hand::escolher_imagem(&texto, &opcoes);
            let anterior = { let fundo = fundo.clone(); tauri::async_runtime::spawn_blocking(move || hand::fundo_atual(&fundo)).await.ok().flatten() };
            card = Some(json!({"home": home, "tipo":"papel_de_parede","id": random_id(), "opcoes": opcoes, "escolhida": escolhida, "programa": fundo.nome(), "anterior": anterior}));
            format!("Escolhi {} para o papel de parede. Confira{} e clique em **Definir**; dá para voltar ao anterior depois.",
                opcoes[escolhida].file_name().map(|n| format!("**{}**", n.to_string_lossy())).unwrap_or_default(), if opcoes.len() > 1 { " (ou troque por outra das opções)" } else { "" })
        }
        Pedido::Tarefa(objetivo) => {
            status("Montando o plano…");
            let (id, mut plano) = planejar_tarefa(state, &objetivo, &contexto.texto()).await?;
            // Programa inventado pelo modelo: não oferece o comando; pergunta como a pessoa faz.
            if let Some(falta) = hand_agent::programa_inexistente(&plano.comando, hand_agent::no_path) {
                if plano.passos.is_empty() {
                    if let Ok(mut tarefas) = TAREFAS.lock() { if let Some(map) = tarefas.as_mut() { map.remove(&id); } }
                    return Ok(Some((None, format!("O jeito que encontrei usa o programa `{falta}`, que não está instalado neste computador. Qual programa você usa para isso? Diga o nome dele, ou o comando, e eu monto de novo."))));
                }
                plano.comando.clear();
            }
            // Dá para fazer sem a tela: um comando, mostrado antes e só rodado com a confirmação.
            if !plano.comando.trim().is_empty() {
                card = Some(json!({"home": home, "tipo":"comando","id": random_id(), "comando": plano.comando.trim()}));
                return Ok(Some((card, "Dá para fazer isso sem usar a tela. Confira o comando abaixo e clique em **Rodar** para ele rodar no seu computador (na pasta pessoal, por até 60 s).".to_string())));
            }
            // Falta saber algo antes de qualquer passo: pergunta, sem cartão.
            if plano.passos.is_empty() {
                if let Ok(mut tarefas) = TAREFAS.lock() { if let Some(map) = tarefas.as_mut() { map.remove(&id); } }
                return Ok(Some((None, format!("Antes de mexer no computador, preciso saber: {} Se forem arquivos que você salvou na Ayla, diga “da Iris” (ex.: “põe os wallpapers da Iris na pasta Wallpapers”).", plano.precisa))));
            }
            let n = plano.passos.len();
            card = Some(json!({"home": home, "tipo":"tarefa","id": id, "objetivo": objetivo, "passos": plano.passos, "precisa": plano.precisa}));
            format!("Montei um plano de {n} {} para fazer isso no computador. Confira e clique em **Aprovar e começar**: eu sigo sozinha e paro para perguntar antes de algo que não dá para desfazer.", if n == 1 { "passo" } else { "passos" })
        }
        Pedido::Comando(comando) => {
            card = Some(json!({"home": home, "tipo":"comando","id": random_id(), "comando": comando}));
            "Confira o comando abaixo e clique em **Rodar** para ele rodar no seu computador (na pasta pessoal, por até 60 s).".to_string()
        }
    };
    Ok(Some((card, reply)))
}

// ---------------------------------------------------------------------------------------------
// Controle: janelas, teclado e mouse (página do Hand; cada ação é um clique da pessoa).
use crate::core::hand_input::{self, Botao};

fn input_dir(state: &Core) -> PathBuf { state.root.join("hand") }

#[tauri::command]
pub fn hand_janelas() -> Result<Vec<hand_input::Janela>, String> { hand_input::janelas() }

#[tauri::command]
pub fn hand_focar(janela: String) -> Result<(), String> { hand_input::focar(&janela) }

#[tauri::command]
pub fn hand_fechar(janela: String) -> Result<(), String> { hand_input::fechar(&janela) }

/// Traz a janela (se dada) e espera ela receber o foco antes de digitar ou clicar.
fn preparar(janela: &Option<String>, espera: Option<u64>) -> Result<(), String> {
    if let Some(address) = janela { hand_input::focar(address)?; std::thread::sleep(std::time::Duration::from_millis(250)); }
    if let Some(seconds) = espera.filter(|s| *s > 0) { std::thread::sleep(std::time::Duration::from_secs(seconds.min(10))); }
    Ok(())
}

#[tauri::command]
pub async fn hand_digitar(texto: String, janela: Option<String>, espera: Option<u64>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || { preparar(&janela, espera)?; hand_input::digitar(&texto) }).await.map_err(|_| "O teclado parou".to_string())?
}

#[tauri::command]
pub async fn hand_teclas(atalho: String, janela: Option<String>, espera: Option<u64>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || { preparar(&janela, espera)?; hand_input::teclas(&atalho) }).await.map_err(|_| "O teclado parou".to_string())?
}

#[tauri::command]
pub async fn hand_clicar(state: tauri::State<'_, Core>, botao: Botao, x: Option<i64>, y: Option<i64>, vezes: Option<u8>, espera: Option<u64>) -> Result<(), String> {
    let dir = input_dir(&state);
    let ponto = x.zip(y);
    tauri::async_runtime::spawn_blocking(move || { preparar(&None, espera)?; hand_input::clicar(&dir, botao, ponto, vezes.unwrap_or(1)) }).await.map_err(|_| "O mouse parou".to_string())?
}

#[tauri::command]
pub async fn hand_rolar(state: tauri::State<'_, Core>, dentes: i32, espera: Option<u64>) -> Result<(), String> {
    let dir = input_dir(&state);
    tauri::async_runtime::spawn_blocking(move || { preparar(&None, espera)?; hand_input::rolar(&dir, dentes) }).await.map_err(|_| "O mouse parou".to_string())?
}

/// Posição do cursor (depois de `espera` segundos, para a pessoa apontar o lugar).
#[tauri::command]
pub async fn hand_cursor(espera: Option<u64>) -> Result<[i64; 2], String> {
    tauri::async_runtime::spawn_blocking(move || { preparar(&None, espera)?; hand_input::cursor().map(|(x, y)| [x, y]) }).await.map_err(|_| "O Hand parou".to_string())?
}

/// Ao fechar a Ayla: encerra o mouse virtual dela.
pub fn hand_encerrar(state: &Core) { hand_input::parar_mouse(&input_dir(state)); }

// ---------------------------------------------------------------------------------------------
// Tarefas: a Ayla opera o computador olhando a tela (plano aprovado → segue sozinha → pergunta
// antes de algo irreversível). Uma por vez; parar pelo atalho global ou pelo botão.
use crate::core::hand_agent::{self, Acao, Plano, Tipo as Ato};

struct Tarefa { objetivo: String, plano: Vec<String>, feitos: Vec<String>, pendente: Option<Acao>, passos: usize }

static TAREFAS: Mutex<Option<HashMap<String, Tarefa>>> = Mutex::new(None);

fn with_tarefa<T>(id: &str, f: impl FnOnce(&mut Tarefa) -> T) -> Result<T, String> {
    let mut all = TAREFAS.lock().map_err(|_| "Tarefas ocupadas")?;
    all.get_or_insert_with(HashMap::new).get_mut(id).map(f).ok_or_else(|| "Essa tarefa não está mais na memória; peça de novo".to_string())
}

fn janelas_texto() -> String {
    hand_input::janelas().map(|list| list.iter().take(12).map(|j| format!("{} ({})", j.title, j.class)).collect::<Vec<_>>().join("; ")).unwrap_or_default()
}

/// Pastas do dia a dia com as subpastas, para o plano usar caminhos reais ("pasta de wallpapers").
fn pastas_texto() -> String {
    hand::default_folders().iter().map(|folder| {
        let mut subs: Vec<String> = fs::read_dir(folder).into_iter().flatten().flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| !n.starts_with('.')).collect();
        subs.sort();
        subs.truncate(30);
        format!("{} (subpastas: {})", folder.display(), if subs.is_empty() { "nenhuma".to_string() } else { subs.join(", ") })
    }).collect::<Vec<_>>().join("; ")
}

/// Monta o plano (modelo local de texto). Nada acontece na tela até a pessoa aprovar.
pub(super) async fn planejar_tarefa(state: &Core, objetivo: &str, contexto: &str) -> Result<(String, Plano), String> {
    let prefs = state.prefs()?;
    let model = models::choose(&state.models()?, Capability::Text, state.budget(&prefs), prefs.context_tokens, None)
        .map_err(|_| "Para operar o computador, prepare um modelo local em Configurações")?;
    let janelas = tauri::async_runtime::spawn_blocking(janelas_texto).await.unwrap_or_default();
    let pastas = tauri::async_runtime::spawn_blocking(pastas_texto).await.unwrap_or_default();
    let token = CancellationToken::new();
    let raw = state.runtime.complete_json_sized(&model, &prefs, hand_agent::plano_pedido(objetivo, &janelas, &pastas, contexto), hand_agent::plano_schema(), 600, 0.2, &token).await?;
    let plano = hand_agent::ler_plano(&raw).ok_or("Não consegui montar um plano para isso")?;
    let id = random_id();
    TAREFAS.lock().map_err(|_| "Tarefas ocupadas")?.get_or_insert_with(HashMap::new)
        .insert(id.clone(), Tarefa { objetivo: objetivo.to_string(), plano: plano.passos.clone(), feitos: Vec::new(), pendente: None, passos: 0 });
    Ok((id, plano))
}

/// Hand › Pedir uma tarefa: o que dá sem a tela (Iris, organizar, buscar, comando) sai direto com
/// prévia; o resto vira um plano para aprovar. Devolve o texto e o cartão, como na conversa.
#[tauri::command]
pub async fn hand_tarefa_planejar(state: tauri::State<'_, Core>, objetivo: String) -> Result<Value, String> {
    let objetivo = objetivo.trim();
    if objetivo.is_empty() { return Err("Diga o que a Ayla deve fazer".into()); }
    let direto = hand::pedido(objetivo).filter(|p| !matches!(p, hand::Pedido::Tarefa(_)));
    if let Some(pedido) = direto {
        if let Some((card, texto)) = resolver(&state, pedido, &Contexto::default(), &|_| {}).await? { return Ok(json!({"texto": texto, "card": card})); }
    }
    let (card, texto) = resolver(&state, hand::Pedido::Tarefa(objetivo.to_string()), &Contexto::default(), &|_| {}).await?.ok_or("Não consegui montar um plano para isso")?;
    Ok(json!({"texto": texto, "card": card}))
}

/// Faz uma ação na tela.
fn agir(dir: &std::path::Path, monitor: &hand_agent::Monitor, acao: &Acao) -> Result<(), String> {
    let ponto = || Some(hand_agent::para_tela(monitor, acao.x, acao.y));
    match acao.acao {
        Ato::Clicar => hand_input::clicar(dir, Botao::Esquerdo, ponto(), 1),
        Ato::CliqueDuplo => hand_input::clicar(dir, Botao::Esquerdo, ponto(), 2),
        Ato::CliqueDireito => hand_input::clicar(dir, Botao::Direito, ponto(), 1),
        Ato::Digitar => hand_input::digitar(&acao.texto),
        Ato::Teclas => hand_input::teclas(&acao.texto),
        Ato::Rolar => hand_input::rolar(dir, if hand::fold(&acao.texto).contains("cima") { -5 } else { 5 }),
        Ato::AbrirPrograma => {
            let programas = hand::programs();
            let programa = hand::programa_pelo_nome(&acao.texto, &programas).ok_or_else(|| format!("Não achei o programa “{}”", acao.texto))?;
            hand::launch(&programa.id)?;
            std::thread::sleep(std::time::Duration::from_millis(1500));
            Ok(())
        }
        Ato::Esperar => { std::thread::sleep(std::time::Duration::from_millis(1500)); Ok(()) }
        Ato::Concluido | Ato::Perguntar => Ok(()),
    }
}

/// O ciclo: olhar → decidir → agir, até concluir, precisar da pessoa, parar ou chegar ao limite.
async fn ciclo(state: &Core, id: &str, events: &Channel<Value>, token: &CancellationToken) -> Result<Value, String> {
    let prefs = state.prefs()?;
    let model = models::choose(&state.models()?, Capability::Vision, state.budget(&prefs), prefs.context_tokens, None)
        .map_err(|_| "Para olhar a tela, prepare um modelo local com visão em Configurações")?;
    let dir = input_dir(state);
    fs::create_dir_all(&dir).map_err(|_| "Não foi possível preparar o Hand")?;
    let tela = dir.join("tela.jpg");
    loop {
        if token.is_cancelled() { return Ok(json!({"estado":"parada","mensagem":"Parei."})); }
        let (objetivo, plano, feitos, passos) = with_tarefa(id, |t| (t.objetivo.clone(), t.plano.clone(), t.feitos.clone(), t.passos))?;
        if passos >= hand_agent::MAX_PASSOS { return Ok(json!({"estado":"limite","mensagem":format!("Parei depois de {passos} ações (o limite por tarefa).")})); }
        let _ = events.send(json!({"type":"olhando"}));
        let monitor = { let tela = tela.clone(); tauri::async_runtime::spawn_blocking(move || { let m = hand_agent::monitor_em_foco()?; hand_agent::olhar(&m, &tela)?; Ok::<_, String>(m) }).await.map_err(|_| "A captura parou")?? };
        let factor = (1280.0 / monitor.width).min(1.0);
        let (largura, altura) = ((monitor.width * factor).round() as u32, (monitor.height * factor).round() as u32);
        let prompt = hand_agent::acao_pedido(&objetivo, &plano, &feitos, largura, altura);
        let messages = vec![
            json!({"role":"system","content":"Você é a Ayla operando o computador da pessoa com teclado e mouse, uma ação por vez, olhando a tela. Responda só com o JSON pedido."}),
            json!({"role":"user","content": with_images(&prompt, std::slice::from_ref(&tela))?}),
        ];
        let raw = state.runtime.complete_json_sized(&model, &prefs, messages, hand_agent::acao_schema(), 400, 0.1, token).await
            .map_err(|e| if token.is_cancelled() { "Parei.".to_string() } else { e })?;
        let _ = fs::remove_file(&tela);
        if token.is_cancelled() { return Ok(json!({"estado":"parada","mensagem":"Parei."})); }
        let acao = hand_agent::ler_acao(&raw).ok_or("O modelo não respondeu com uma ação válida")?;
        let texto = hand_agent::descrever(&acao);
        if acao.acao == Ato::Concluido {
            return Ok(json!({"estado":"concluida","mensagem": if acao.motivo.trim().is_empty() { "Pronto.".to_string() } else { acao.motivo.trim().to_string() }}));
        }
        if hand_agent::precisa_aprovar(&acao) {
            with_tarefa(id, |t| t.pendente = Some(acao.clone()))?;
            let pergunta = if acao.acao == Ato::Perguntar { acao.motivo.clone() } else { format!("Posso fazer isto? {texto}") };
            return Ok(json!({"estado":"pergunta","mensagem": pergunta, "acao": texto, "so_pergunta": acao.acao == Ato::Perguntar}));
        }
        let feito = { let (dir, monitor, acao) = (dir.clone(), monitor.clone(), acao.clone()); tauri::async_runtime::spawn_blocking(move || agir(&dir, &monitor, &acao)).await.map_err(|_| "O Hand parou")? };
        let linha = match &feito { Ok(()) => texto.clone(), Err(e) => format!("{texto} (falhou: {e})") };
        with_tarefa(id, |t| { t.feitos.push(linha.clone()); t.passos += 1; })?;
        let _ = events.send(json!({"type":"passo","texto": linha}));
        tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    }
}

/// Em volta do ciclo: uma tarefa por vez, atalho global para parar e aviso do sistema.
async fn rodar(state: &Core, id: &str, events: &Channel<Value>) -> Result<Value, String> {
    let (numero, token) = hand_agent::comecar();
    let binario = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("ayla"));
    let atalho = tauri::async_runtime::spawn_blocking(move || hand_agent::registrar_atalho(&binario)).await.ok().flatten();
    let aviso = atalho.as_ref().map(|(_, label)| format!("{label} para parar")).unwrap_or_else(|| "Use o botão Parar na Ayla".into());
    let _ = events.send(json!({"type":"comecou","atalho": atalho.as_ref().map(|(_, l)| l.clone())}));
    #[cfg(target_os = "linux")]
    { let _ = notify_rust::Notification::new().appname("Ayla").summary("A Ayla está usando o computador").body(&aviso).icon(&crate::desktop::notification_icon()).timeout(5000).show(); }
    let _ = &aviso;
    let result = ciclo(state, id, events, &token).await;
    hand_agent::terminar(numero);
    if let Some((mods, _)) = atalho { let _ = tauri::async_runtime::spawn_blocking(move || hand_agent::tirar_atalho(&mods)).await; }
    result
}

/// Aprovado o plano: a Ayla começa a operar. Devolve como terminou (concluída, pergunta, parada…).
#[tauri::command]
pub async fn hand_tarefa_executar(state: tauri::State<'_, Core>, id: String, events: Channel<Value>) -> Result<Value, String> {
    rodar(&state, &id, &events).await
}

/// Resposta à pergunta da Ayla: `aprovar` faz a ação pendente e segue; senão, para.
#[tauri::command]
pub async fn hand_tarefa_responder(state: tauri::State<'_, Core>, id: String, aprovar: bool, events: Channel<Value>) -> Result<Value, String> {
    let pendente = with_tarefa(&id, |t| t.pendente.take())?;
    if !aprovar { return Ok(json!({"estado":"parada","mensagem":"Ok, parei sem fazer."})); }
    if let Some(acao) = pendente.filter(|a| a.acao != Ato::Perguntar) {
        let dir = input_dir(&state);
        let monitor = tauri::async_runtime::spawn_blocking(hand_agent::monitor_em_foco).await.map_err(|_| "O Hand parou")??;
        let texto = hand_agent::descrever(&acao);
        let feito = tauri::async_runtime::spawn_blocking({ let dir = dir.clone(); move || agir(&dir, &monitor, &acao) }).await.map_err(|_| "O Hand parou")?;
        let linha = match &feito { Ok(()) => texto, Err(e) => format!("{texto} (falhou: {e})") };
        with_tarefa(&id, |t| { t.feitos.push(linha.clone()); t.passos += 1; })?;
        let _ = events.send(json!({"type":"passo","texto": linha}));
    } else {
        with_tarefa(&id, |t| t.feitos.push("A pessoa respondeu e pediu para seguir".into()))?;
    }
    rodar(&state, &id, &events).await
}

#[tauri::command]
pub fn hand_tarefa_parar() -> bool { hand_agent::parar() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contexto_da_conversa_para_o_proximo_pedido() {
        let dir = std::env::temp_dir().join(format!("ayla-hand-ctx-{}", store::id()));
        fs::create_dir_all(&dir).unwrap();
        let (movida, parada) = (dir.join("a.png"), dir.join("b.jpg"));
        fs::write(&movida, b"x").unwrap(); fs::write(&parada, b"x").unwrap();
        let plano = json!({"hand": {"tipo": "plano", "passos": [
            {"acao": "iris", "arquivo": "1", "nome": "a.png", "para": movida},
            {"acao": "mover", "de": parada, "para": dir.join("sub/b.jpg")},
            {"acao": "mover", "de": dir.join("sumiu.png"), "para": dir.join("x/sumiu.png")}]}});
        let ctx = Contexto::de(&[json!({"ficha": {}}), plano]);
        assert_eq!(ctx.arquivos, vec![movida.clone(), parada.clone()], "movido no destino, não aplicado na origem, sumido fora");
        assert!(ctx.pergunta.is_none() && ctx.texto().contains("a.png"));
        let pergunta = Contexto::de(&[json!({"hand": {"tipo": "pergunta", "objetivo": "move os wallpapers que eu baixei para a pasta wallpapers"}})]);
        assert!(pergunta.arquivos.is_empty() && pergunta.pergunta.is_some());
        assert!(responde_origem("pela ayla") && responde_origem("foi pelo navegador") && !responde_origem("obrigado") && !responde_origem("oi tudo bem?"));
        // "pela ayla" + o pedido anterior = Iris.
        assert!(matches!(hand::pedido(&format!("pela ayla, {}", pergunta.pergunta.unwrap())), Some(hand::Pedido::DaIris { .. })));
        assert!(Contexto::de(&[]).arquivos.is_empty());
        let fundo = Contexto::de(&[json!({"hand": {"tipo": "papel_de_parede", "opcoes": [movida, parada]}})]);
        assert_eq!((fundo.tipo.as_str(), fundo.arquivos.len()), ("papel_de_parede", 2));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn iris_da_ayla_sai_do_registro_e_volta_ao_desfazer() {
        let root = std::env::temp_dir().join(format!("ayla-hand-iris-{}", store::id()));
        let core = Core::open(root.join("dados")).unwrap();
        let png = b"\x89PNG\r\n\x1a\n0000000000";
        let add = |name: &str| core.store_file(name, |dest| fs::write(dest, png).map(|_| png.len() as u64).map_err(|e| e.to_string()), None, true).unwrap();
        let (a, b) = (add("praia.png"), add("montanha.png"));
        fs::write(core.root.join("files").join(&a.id).join("creditos.json"), r#"{"autor":"x"}"#).unwrap();
        let destino = root.join("Pictures/Wallpapers");
        fs::create_dir_all(&destino).unwrap();
        let plan = hand::plan_from_iris(&[(a.id.clone(), a.name.clone()), (b.id.clone(), b.name.clone())], &destino);
        let iris = IrisDaAyla(&core);
        let feito = hand::apply(&plan, &[root.join("Pictures")], "Iris → Wallpapers", Some(&iris)).unwrap();
        assert!(feito.falhas.is_empty(), "{:?}", feito.falhas);
        assert!(destino.join("praia.png").is_file() && destino.join("montanha.png").is_file());
        assert!(core.store.lock().unwrap().file(&a.id).unwrap().is_none(), "saiu do registro");
        assert!(!core.root.join("files").join(&a.id).exists(), "e da pasta de dados, com os créditos");
        assert!(core.store.lock().unwrap().media_files().unwrap().is_empty(), "a Iris não mostra mais");
        let (voltaram, problemas) = hand::undo(&feito.lote, Some(&iris));
        assert_eq!((voltaram, problemas.len()), (2, 0), "{problemas:?}");
        let de_volta = core.store.lock().unwrap().file(&a.id).unwrap().unwrap();
        assert_eq!((de_volta.name.as_str(), de_volta.path.clone()), ("praia.png", format!("files/{}/praia.png", a.id)));
        assert_eq!(fs::read_to_string(core.root.join("files").join(&a.id).join("creditos.json")).unwrap(), r#"{"autor":"x"}"#);
        assert_eq!(core.store.lock().unwrap().media_files().unwrap().len(), 2, "voltou para a Iris");
        assert!(!destino.join("praia.png").exists());
        let _ = fs::remove_dir_all(&root);
    }
}
