//! Feriados no calendário: os da cidade da pessoa (pesquisa anual pela nuvem, guardada aqui) e o
//! estado da busca para Configurações › Athena › Calendários.
use super::*;
use crate::core::feriados::{self, Pesquisa};

fn pasta(state: &Core) -> PathBuf { state.root.join("feriados") }

pub(super) fn guardada(state: &Core, uf: &str, cidade: &str, ano: i32) -> Option<Pesquisa> {
    if uf.is_empty() || cidade.trim().is_empty() { return None; }
    fs::read(pasta(state).join(feriados::chave(uf, cidade, ano))).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

static BUSCANDO: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
static ERROS: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

/// A pesquisa dos municipais precisa de uma nuvem com busca na web e de um modo que permita a nuvem.
fn nuvem(prefs: &Preferences) -> Result<String, String> {
    if prefs.usage_mode == UsageMode::LocalOnly { return Err("A Ayla está em \"só neste computador\": os feriados da cidade não são pesquisados.".into()); }
    let platform = prefs.cloud_platform().ok_or("Para os feriados da cidade, conecte o ChatGPT ou a OpenAI em Configurações › Nuvem.")?;
    if !matches!(platform, "chatgpt" | "openai") { return Err("A pesquisa dos feriados da cidade usa o ChatGPT ou a OpenAI (Configurações › Nuvem).".into()); }
    Ok(platform.to_string())
}

/// Pesquisa os estaduais e municipais do ano (só o nome da cidade e do estado saem) e guarda.
async fn buscar(state: &Core, uf: &str, cidade: &str, ano: i32) -> Result<Pesquisa, String> {
    let prefs = state.prefs()?;
    let platform = nuvem(&prefs)?;
    let (api, base, secret) = state.cloud_secret(&prefs, &platform).await?;
    let model = state.cloud_auto_model(&prefs, &platform, api, &base, &secret).await?;
    let request = cloud::search_request(&platform, &secret, &model, None, "Você consulta feriados oficiais brasileiros em fontes públicas (prefeituras, assembleias, leis).", &[], &feriados::pedido(uf, cidade, ano))?;
    let text = cloud::search_text(request, &CancellationToken::new()).await?;
    let (estaduais, municipais) = feriados::ler_pesquisa(&text, ano).ok_or("A pesquisa não trouxe uma lista de feriados")?;
    let pesquisa = Pesquisa { buscado_em: store::now(), estaduais, municipais };
    fs::create_dir_all(pasta(state)).map_err(|_| "Não foi possível guardar os feriados")?;
    fs::write(pasta(state).join(feriados::chave(uf, cidade, ano)), serde_json::to_vec_pretty(&pesquisa).unwrap_or_default()).map_err(|_| "Não foi possível guardar os feriados")?;
    Ok(pesquisa)
}

/// Busca em segundo plano (uma por vez para cada cidade e ano) e avisa o calendário ao terminar.
pub(super) fn agendar(app: &tauri::AppHandle, uf: String, cidade: String, ano: i32) {
    let key = feriados::chave(&uf, &cidade, ano);
    {
        let Ok(mut b) = BUSCANDO.lock() else { return };
        if b.contains(&key) { return; }
        // Já falhou nesta sessão: não insiste sozinha (o botão "Buscar agora" tenta de novo).
        if ERROS.lock().is_ok_and(|e| e.iter().any(|(k, _)| *k == key)) { return; }
        b.push(key.clone());
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        use tauri::{Emitter, Manager};
        let result = match app.try_state::<Core>() { Some(state) => buscar(&state, &uf, &cidade, ano).await.map(|_| ()), None => Ok(()) };
        if let Ok(mut b) = BUSCANDO.lock() { b.retain(|k| *k != key); }
        if let Ok(mut e) = ERROS.lock() { e.retain(|(k, _)| *k != key); if let Err(err) = result { e.push((key, err)); } }
        let _ = app.emit("reminders-updated", json!({}));
        let _ = app.emit("holidays-updated", json!({}));
    });
}

/// Feriados entre `from` e `to` para o calendário; agenda a pesquisa dos anos que faltam.
pub(super) fn do_periodo(app: &tauri::AppHandle, state: &Core, prefs: &Preferences, from: chrono::NaiveDate, to: chrono::NaiveDate) -> Vec<feriados::Feriado> {
    if !prefs.calendar.feriados { return Vec::new(); }
    let (uf, cidade) = (prefs.calendar.uf.clone(), prefs.calendar.cidade.trim().to_string());
    let uf_opt = (!uf.is_empty()).then_some(uf.as_str());
    if uf_opt.is_some() && !cidade.is_empty() && nuvem(prefs).is_ok() {
        use chrono::Datelike;
        for ano in from.year()..=to.year() {
            if guardada(state, &uf, &cidade, ano).is_none() { agendar(app, uf.clone(), cidade.clone(), ano); }
        }
    }
    feriados::periodo(from, to, uf_opt, &|ano| guardada(state, &uf, &cidade, ano))
}

/// Estado para as configurações: local, o que foi pesquisado no ano, se dá para pesquisar e erros.
#[tauri::command]
pub fn feriados_estado(state: tauri::State<'_, Core>, ano: i32) -> Result<Value, String> {
    let prefs = state.prefs()?;
    let (uf, cidade) = (prefs.calendar.uf.clone(), prefs.calendar.cidade.trim().to_string());
    let key = feriados::chave(&uf, &cidade, ano);
    let ufs: Vec<Value> = feriados::UFS.iter().map(|(s, n)| json!({"sigla": s, "nome": n})).collect();
    Ok(json!({
        "ufs": ufs,
        "pesquisa": guardada(&state, &uf, &cidade, ano),
        "estaduais_embutidos": if uf.is_empty() { Vec::new() } else { feriados::estaduais(&uf, ano) },
        "nuvem": nuvem(&prefs).err(),
        "buscando": BUSCANDO.lock().is_ok_and(|b| b.contains(&key)),
        "erro": ERROS.lock().ok().and_then(|e| e.iter().find(|(k, _)| *k == key).map(|(_, m)| m.clone())),
    }))
}

/// "Buscar agora": pesquisa de novo os feriados da cidade no ano (mesmo que já tenha).
#[tauri::command]
pub async fn feriados_buscar(app: tauri::AppHandle, state: tauri::State<'_, Core>, ano: i32) -> Result<Value, String> {
    use tauri::Emitter;
    let prefs = state.prefs()?;
    let (uf, cidade) = (prefs.calendar.uf.clone(), prefs.calendar.cidade.trim().to_string());
    if uf.is_empty() || cidade.is_empty() { return Err("Escolha o estado e escreva a cidade primeiro.".into()); }
    let key = feriados::chave(&uf, &cidade, ano);
    if let Ok(mut e) = ERROS.lock() { e.retain(|(k, _)| *k != key); }
    let pesquisa = buscar(&state, &uf, &cidade, ano).await;
    if let Err(err) = &pesquisa { if let Ok(mut e) = ERROS.lock() { e.push((key, err.clone())); } }
    let _ = app.emit("reminders-updated", json!({}));
    Ok(json!(pesquisa?))
}
