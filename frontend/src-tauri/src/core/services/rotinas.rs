//! Rotinas: comandos da página (Hand › Rotinas), o agendador (a cada 20 s, com a Ayla aberta ou em
//! segundo plano), a execução (condições, preço, ações) e a criação pela conversa.
use super::*;
use crate::core::rotinas::{self, Acao, Compara, Condicao, Execucao, Rotina};
use std::sync::atomic::{AtomicBool, Ordering};

fn dados(state: &Core) -> PathBuf { state.root.join("hand") }
/// Definições na configuração (dotfiles); se a pasta não pôde ser criada, o caminho mesmo assim.
fn config() -> PathBuf { crate::core::ayla_config::preparar().unwrap_or_else(|_| crate::core::ayla_config::pasta()) }

fn novo_id() -> String {
    format!("r-{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0))
}

fn agora() -> chrono::NaiveDateTime { reminders::now_local() }
fn local_s(t: chrono::NaiveDateTime) -> i64 { t.and_utc().timestamp() }

/// Rotinas com a próxima execução e o histórico.
#[tauri::command]
pub fn rotinas_lista(state: tauri::State<'_, Core>) -> Result<Value, String> {
    let dir = dados(&state);
    let historico = rotinas::historico(&dir);
    let now = agora();
    let list: Vec<Value> = rotinas::carregar(&config(), &dir).into_iter().map(|r| {
        let mut v = json!(r);
        v["resumo"] = json!(rotinas::resumo(&r));
        v["proxima"] = json!(rotinas::proxima(&r, now));
        v["historico"] = json!(historico.get(&r.id).map(|h| h.iter().rev().take(60).cloned().collect::<Vec<_>>()).unwrap_or_default());
        v
    }).collect();
    let nuvem = state.prefs().ok().and_then(|p| p.cloud_platform().map(str::to_string));
    Ok(json!({"rotinas": list, "nuvem": nuvem, "agora": local_s(now)}))
}

/// Cria ou atualiza. Um comando só fica aprovado se a página mandou `aprovado` igual ao comando.
#[tauri::command]
pub fn rotina_salvar(state: tauri::State<'_, Core>, item: Rotina) -> Result<(), String> {
    rotinas::validar(&item)?;
    let dir = dados(&state);
    let mut list = rotinas::carregar(&config(), &dir);
    let mut item = item;
    match list.iter_mut().find(|r| r.id == item.id) {
        Some(existing) => {
            // O estado do agendador continua o mesmo (a não ser que o gatilho mude).
            if existing.gatilho == item.gatilho { item.ultima = existing.ultima; }
            item.ultimo_disparo = existing.ultimo_disparo;
            item.ultimo_preco = existing.ultimo_preco;
            *existing = item;
        }
        None => { item.ultima = None; item.ultimo_disparo = None; item.ultimo_preco = None; list.push(item); }
    }
    rotinas::guardar(&config(), &dir, &list)
}

#[tauri::command]
pub fn rotina_ativar(state: tauri::State<'_, Core>, id: String, ativa: bool) -> Result<(), String> {
    let dir = dados(&state);
    let mut list = rotinas::carregar(&config(), &dir);
    let r = list.iter_mut().find(|r| r.id == id).ok_or("Rotina não encontrada")?;
    if ativa { rotinas::validar(r)?; }
    r.ativa = ativa;
    rotinas::guardar(&config(), &dir, &list)
}

#[tauri::command]
pub fn rotina_apagar(state: tauri::State<'_, Core>, id: String) -> Result<(), String> {
    let dir = dados(&state);
    let mut list = rotinas::carregar(&config(), &dir);
    list.retain(|r| r.id != id);
    rotinas::guardar(&config(), &dir, &list)?;
    rotinas::esquecer_historico(&dir, &id)
}

/// Roda agora (botão "Testar agora"), mesmo pausada; o histórico registra.
#[tauri::command]
pub async fn rotina_testar(app: tauri::AppHandle, state: tauri::State<'_, Core>, id: String) -> Result<Execucao, String> {
    let r = rotinas::carregar(&config(), &dados(&state)).into_iter().find(|r| r.id == id).ok_or("Rotina não encontrada")?;
    Ok(executar(&app, &state, r).await)
}

// ---------------------------------------------------------------------------------------------
// Agendador

static RODANDO: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
static PRIMEIRA: AtomicBool = AtomicBool::new(true);

/// A sessão começou há pouco (ligou o PC / entrou agora)? Pelo `loginctl` ou pelo Hyprland.
fn sessao_recente() -> bool {
    let uptime = std::fs::read_to_string("/proc/uptime").ok().and_then(|t| t.split_whitespace().next().and_then(|v| v.parse::<f64>().ok())).unwrap_or(0.0);
    let inicio = std::env::var("XDG_SESSION_ID").ok().and_then(|id| std::process::Command::new("loginctl").args(["show-session", &id, "-p", "TimestampMonotonic", "--value"]).output().ok())
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().ok()).map(|us| us / 1_000_000.0)
        .or_else(|| {
            // Início do compositor (campo 22 do /proc/<pid>/stat, em ticks de 1/100 s).
            let pid = std::process::Command::new("pidof").arg("Hyprland").output().ok()?;
            let pid = String::from_utf8_lossy(&pid.stdout).split_whitespace().next()?.to_string();
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            let after = stat.rsplit_once(')')?.1;
            after.split_whitespace().nth(19)?.parse::<f64>().ok().map(|t| t / 100.0)
        });
    inicio.is_some_and(|s| s > 0.0 && uptime - s < 20.0 * 60.0)
}

/// Confere as rotinas que venceram e roda cada uma em paralelo (uma vez por vez cada).
pub async fn rotinas_tick(app: &tauri::AppHandle) {
    use tauri::Manager;
    let Some(state) = app.try_state::<Core>() else { return };
    let entrou = PRIMEIRA.swap(false, Ordering::SeqCst) && sessao_recente();
    let now = agora();
    for r in rotinas::carregar(&config(), &dados(&state)) {
        if !rotinas::vencida(&r, now, entrou) { continue; }
        {
            let Ok(mut rodando) = RODANDO.lock() else { continue };
            if rodando.contains(&r.id) { continue; }
            rodando.push(r.id.clone());
        }
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let id = r.id.clone();
            if let Some(state) = app.try_state::<Core>() { let _ = executar(&app, &state, r).await; }
            if let Ok(mut rodando) = RODANDO.lock() { rodando.retain(|x| *x != id); }
        });
    }
}

// ---------------------------------------------------------------------------------------------
// Executar

fn programa_aberto(nome: &str) -> bool {
    let alvo = crate::core::hand::fold(nome.trim());
    if alvo.is_empty() { return false; }
    let janelas = crate::core::hand_input::janelas().unwrap_or_default();
    if janelas.iter().any(|j| crate::core::hand::fold(&j.class).contains(&alvo) || crate::core::hand::fold(&j.title).contains(&alvo)) { return true; }
    std::fs::read_dir("/proc").into_iter().flatten().flatten().any(|e| {
        e.file_name().to_string_lossy().chars().all(|c| c.is_ascii_digit())
            && std::fs::read_to_string(e.path().join("comm")).is_ok_and(|c| crate::core::hand::fold(c.trim()).contains(&alvo))
    })
}

/// Preço atual: pela página (regras e, se preciso, modelo local) e, com a nuvem liberada, pela
/// pesquisa da nuvem quando a página não mostra.
async fn checar_preco(state: &Core, r: &Rotina, url: &str, produto: &str) -> Result<f64, String> {
    let mut erro = String::new();
    if !url.trim().is_empty() {
        match baixar_pagina(url).await {
            Ok(html) => {
                if let Some(p) = rotinas::preco_do_html(&html) { return Ok(p); }
                let texto = rotinas::texto_visivel(&html, 6000);
                if let Ok(prefs) = state.prefs() {
                    if let Ok(model) = state.models().and_then(|m| models::choose(&m, Capability::Text, state.budget(&prefs), prefs.context_tokens, None)) {
                        let token = CancellationToken::new();
                        if let Ok(raw) = state.runtime.complete_json_sized(&model, &prefs, rotinas::preco_pedido(&texto), rotinas::preco_schema(), 60, 0.0, &token).await {
                            if let Some(p) = rotinas::ler_preco_modelo(&raw) { return Ok(p); }
                        }
                    }
                }
                erro = "A página abriu, mas não mostrou o preço para a Ayla".into();
            }
            Err(e) => erro = e,
        }
    }
    if !r.nuvem { return Err(if erro.is_empty() { "Sem link da loja".into() } else { format!("{erro}. Libere a nuvem nesta rotina para a Ayla pesquisar o preço.") }); }
    preco_pela_nuvem(state, url, produto).await.map_err(|e| if erro.is_empty() { e } else { format!("{erro}; pela nuvem: {e}") })
}

async fn baixar_pagina(url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36")
        .connect_timeout(std::time::Duration::from_secs(10)).timeout(std::time::Duration::from_secs(25)).build().map_err(|_| "Falha ao preparar o acesso")?;
    let response = client.get(url).header("Accept-Language", "pt-BR,pt;q=0.9").send().await.map_err(|_| "A loja não respondeu")?;
    if !response.status().is_success() { return Err(format!("A loja recusou o acesso (HTTP {})", response.status().as_u16())); }
    let bytes = response.bytes().await.map_err(|_| "A página não terminou de carregar")?;
    Ok(String::from_utf8_lossy(&bytes[..bytes.len().min(4 << 20)]).into_owned())
}

/// Pesquisa da nuvem (só o link ou o nome do produto vão; nada da pessoa).
async fn preco_pela_nuvem(state: &Core, url: &str, produto: &str) -> Result<f64, String> {
    let prefs = state.prefs()?;
    if prefs.usage_mode == UsageMode::LocalOnly { return Err("A Ayla está em \"só neste computador\"".into()); }
    let platform = prefs.cloud_platform().ok_or("Nenhuma nuvem conectada")?.to_string();
    let (_, _, secret) = state.cloud_secret(&prefs, &platform).await?;
    let model = cloud::provider(&platform).and_then(|p| p.default_model).filter(|m| !m.is_empty()).unwrap_or("gpt-5-mini").to_string();
    let alvo = if url.trim().is_empty() { produto.to_string() } else { format!("{produto} {url}") };
    let request = cloud::search_request(&platform, &secret, &model, None, "Você checa preços para uma rotina automática.", &format!("Qual é o preço atual à vista, em reais, de: {alvo}? Responda só o JSON {{\"preco\": número}}."))?;
    let token = CancellationToken::new();
    let text = cloud::stream_search(request, &token, false, |_| {}).await?;
    rotinas::preco_da_resposta(&text).ok_or_else(|| "A pesquisa não trouxe um preço".into())
}

/// Conversa onde as mensagens das rotinas ficam.
fn conversa_rotinas(state: &Core) -> Result<String, String> {
    let file = dados(state).join("rotinas-conversa.txt");
    let db = state.store.lock().map_err(|_| "Dados ocupados")?;
    if let Some(id) = std::fs::read_to_string(&file).ok().map(|s| s.trim().to_string()).filter(|id| db.conversation_project(id).is_ok()) { return Ok(id); }
    let c = db.create_conversation("Rotinas", None).map_err(|_| "Não foi possível criar a conversa das rotinas")?;
    let _ = std::fs::create_dir_all(dados(state));
    let _ = std::fs::write(&file, &c.id);
    Ok(c.id)
}

async fn mensagem_ayla(app: &tauri::AppHandle, state: &Core, nome: &str, pedido: &str, preco: Option<f64>) -> Result<String, String> {
    use tauri::Emitter;
    let prefs = state.prefs()?;
    let model = models::choose(&state.models()?, Capability::Text, state.budget(&prefs), prefs.context_tokens, None).map_err(|_| "Sem modelo local para escrever a mensagem")?;
    let now = agora();
    let eventos = state.store.lock().map_err(|_| "Dados ocupados")?.events().unwrap_or_default().into_iter()
        .map(|r| (r.text, reminders::from_unix(r.start), reminders::Repeat::parse(&r.repeat), r.all_day, r.alerts)).collect::<Vec<_>>();
    let agenda = reminders::agenda_block(&eventos, now, 1);
    let mut contexto = format!("Agora: {}.{agenda}", now.format("%A, %d/%m/%Y %H:%M"));
    if let Some(p) = preco { contexto.push_str(&format!("\nPreço checado agora: R$ {}.", rotinas::formatar(p))); }
    let messages = vec![
        json!({"role":"system","content": format!("{}\n\nVocê está escrevendo uma mensagem curta de uma rotina automática chamada \"{nome}\" (até 4 frases, em português, sem inventar compromissos: use só a agenda dada). Responda só o JSON {{\"mensagem\": \"...\"}}.", prefs.system_prompt())}),
        json!({"role":"user","content": format!("{pedido}\n\n{contexto}")}),
    ];
    let token = CancellationToken::new();
    let raw = state.runtime.complete_json_sized(&model, &prefs, messages, json!({"type":"object","properties":{"mensagem":{"type":"string"}},"required":["mensagem"]}), 400, 0.6, &token).await?;
    let start = raw.find('{').unwrap_or(0);
    let text = serde_json::from_str::<Value>(&raw[start..]).ok().and_then(|v| v["mensagem"].as_str().map(str::to_string)).filter(|t| !t.trim().is_empty()).ok_or("O modelo não escreveu a mensagem")?;
    let conversa = conversa_rotinas(state)?;
    {
        // Como uma troca na conversa: "Rotina: Bom dia" e a mensagem da Ayla.
        let db = state.store.lock().map_err(|_| "Dados ocupados")?;
        let parent = db.leaf(&conversa).map_err(|_| "Conversa das rotinas não encontrada")?;
        db.save_turn(&conversa, store::Placement { parent, user: None, files_from: None }, &format!("Rotina: {nome}"), &text, true, &[], &[], None)
            .map_err(|_| "Não foi possível guardar a mensagem")?;
    }
    let _ = app.emit("conversation-updated", json!({"id": conversa, "title": "Rotinas"}));
    Ok(text)
}

fn notificar(titulo: &str, texto: &str) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    { notify_rust::Notification::new().appname("Ayla").summary(titulo).body(texto).icon(&crate::desktop::notification_icon()).show().map(|_| ()).map_err(|_| "O sistema não mostrou a notificação".into()) }
    #[cfg(not(target_os = "linux"))]
    { let _ = (titulo, texto); Ok(()) }
}

/// Confere as condições e, se valerem, roda as ações. Guarda o resultado no histórico e no estado.
pub async fn executar(app: &tauri::AppHandle, state: &Core, r: Rotina) -> Execucao {
    let now = agora();
    let mut resultados = Vec::new();
    let mut detalhes: Vec<String> = Vec::new();
    let mut preco: Option<f64> = None;
    let anterior = r.ultimo_preco;
    let todas = r.condicoes.modo == rotinas::Modo::Todas;
    // Primeiro as locais (baratas); no modo "todas", uma falsa evita acessar a loja.
    for c in &r.condicoes.itens {
        if let Some(ok) = rotinas::condicao_local(c, &r, now, &programa_aberto) { resultados.push(ok); }
    }
    let precisa_preco = !(todas && resultados.iter().any(|ok| !ok));
    if precisa_preco {
        for c in &r.condicoes.itens {
            let Condicao::Preco { url, produto, compara, valor } = c else { continue };
            match checar_preco(state, &r, url, produto).await {
                Ok(p) => {
                    preco = Some(p);
                    let ok = rotinas::compara_preco(*compara, *valor, p, anterior);
                    detalhes.push(format!("Preço: R$ {}{}", rotinas::formatar(p), anterior.map(|a| format!(" (antes R$ {})", rotinas::formatar(a))).unwrap_or_default()));
                    if matches!(compara, Compara::Caiu | Compara::Mudou) && anterior.is_none() { detalhes.push("primeira checagem: guardei o preço para comparar".into()); }
                    resultados.push(ok);
                }
                Err(e) => { detalhes.push(e); resultados.push(false); }
            }
        }
    } else {
        detalhes.push("Condições de horário/dia/programa não valeram".into());
    }
    let disparou = rotinas::juntar(r.condicoes.modo, &resultados);
    if disparou {
        for a in &r.acoes {
            let feito = match a {
                Acao::Notificar { titulo, texto } => {
                    let titulo = if titulo.trim().is_empty() { r.nome.clone() } else { rotinas::preencher(titulo, &r.nome, preco, anterior, now) };
                    notificar(&titulo, &rotinas::preencher(texto, &r.nome, preco, anterior, now)).map(|_| "notificação enviada".to_string())
                }
                Acao::MensagemAyla { pedido } => match mensagem_ayla(app, state, &r.nome, pedido, preco).await {
                    Ok(text) => { let _ = notificar(&r.nome, &text.chars().take(240).collect::<String>()); Ok("mensagem da Ayla na conversa Rotinas".into()) }
                    Err(e) => Err(e),
                },
                Acao::Macro { id, nome } => super::hand_sistema::rodar_macro(dados(state), id.clone()).await.map(|_| format!("macro {} rodou", if nome.is_empty() { id } else { nome })),
                Acao::Comando { comando, aprovado } if aprovado == comando => crate::core::hand::run(comando, &crate::core::hand::home(), std::time::Duration::from_secs(60)).await
                    .map(|s| format!("comando terminou com código {}", s.code.map_or("?".into(), |c| c.to_string()))),
                Acao::Comando { .. } => Err("comando ainda não aprovado (abra a rotina e salve para aprovar)".into()),
            };
            detalhes.push(match feito { Ok(t) => t, Err(e) => format!("falhou: {e}") });
        }
    }
    let execucao = Execucao { quando: local_s(now), disparou, detalhe: detalhes.join(" · "), preco };
    // Estado: relê a lista (a pessoa pode ter editado enquanto rodava).
    let dir = dados(state);
    let mut list = rotinas::carregar(&config(), &dir);
    if let Some(atual) = list.iter_mut().find(|x| x.id == r.id) {
        atual.ultima = Some(local_s(now));
        if disparou { atual.ultimo_disparo = Some(local_s(now)); }
        if preco.is_some() { atual.ultimo_preco = preco; }
        let _ = rotinas::guardar(&config(), &dir, &list);
    }
    let _ = rotinas::registrar(&dir, &r.id, execucao.clone());
    execucao
}

// ---------------------------------------------------------------------------------------------
// Pela conversa

/// "Checa o preço disto a cada 1 hora": o modelo local monta a rotina, que chega desligada num
/// cartão para a pessoa ativar (ou ajustar em Hand › Rotinas). `None` se não for um pedido de rotina.
pub(super) async fn rotina_conversa(state: &Core, message: &str, events: &Channel<Value>, extras: &std::sync::Mutex<Value>) -> Result<Option<String>, String> {
    if !rotinas::pedido(message) { return Ok(None); }
    let prefs = state.prefs()?;
    let Ok(model) = state.models().and_then(|m| models::choose(&m, Capability::Text, state.budget(&prefs), prefs.context_tokens, None)) else {
        return Ok(Some("Para montar a rotina pela conversa preciso de um modelo local. Dá para criar à mão em **Hand › Rotinas**.".into()));
    };
    let _ = events.send(json!({"type":"status","text":"Montando a rotina…"}));
    let token = CancellationToken::new();
    let raw = state.runtime.complete_json_sized(&model, &prefs, rotinas::pedido_modelo(message, agora()), rotinas::pedido_schema(), 700, 0.1, &token).await?;
    let Some(mut r) = rotinas::ler_pedido(&raw, novo_id()) else {
        return Ok(Some("Não consegui montar essa rotina. Tente dizer quando (\"a cada 1 hora\", \"todo dia às 8h\", \"quando eu abrir o PC\") e o que fazer, ou crie em **Hand › Rotinas**.".into()));
    };
    // O link que a pessoa colou vale mais que o do modelo.
    if let Some(link) = message.split_whitespace().find(|w| w.starts_with("https://") || w.starts_with("http://")) {
        for c in r.condicoes.itens.iter_mut() { if let Condicao::Preco { url, .. } = c { *url = link.trim_end_matches(['.', ',', ')']).to_string(); } }
    }
    let falta = rotinas::validar(&r).err();
    let dir = dados(state);
    let mut list = rotinas::carregar(&config(), &dir);
    r.ativa = false;
    list.push(r.clone());
    rotinas::guardar(&config(), &dir, &list)?;
    send_kept(events, extras, json!({"type":"hand","item":{"tipo":"rotina","id": r.id, "nome": r.nome, "resumo": rotinas::resumo(&r), "falta": falta}}));
    let _ = events.send(json!({"type":"routing","text":"Rotina · neste computador"}));
    Ok(Some(match &falta {
        None => format!("Montei a rotina **{}**: {}. Ela está desligada até você ativar.", r.nome, rotinas::resumo(&r)),
        Some(f) => format!("Montei a rotina **{}**, mas falta um detalhe: {f}. Ajuste em **Hand › Rotinas** e ative.", r.nome),
    }))
}
