//! Rotinas (Hand › Rotinas): "checa o preço disto a cada 1 hora", "bom dia quando eu abrir o PC
//! de manhã". Um gatilho (intervalo, horários, ao entrar na sessão), condições combinadas (todas
//! ou qualquer) e ações (notificação, mensagem escrita pela Ayla, macro, comando). Rodam com a Ayla
//! aberta ou em segundo plano. Cada execução fica no histórico (o preço vira gráfico).
use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum Gatilho {
    /// A cada N minutos.
    Intervalo { minutos: u32 },
    /// Em horários ("08:00"), nos dias da semana marcados (0 = segunda … 6 = domingo; vazio = todos).
    Horario { horas: Vec<String>, #[serde(default)] dias: Vec<u8> },
    /// Logo depois de entrar na sessão (ligar o PC).
    AoEntrar,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Modo { #[default] Todas, Qualquer }

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Compara {
    /// Só acompanhar (sempre vale; o preço vai para o histórico).
    Sempre,
    Abaixo,
    Acima,
    /// Menor que na checagem anterior.
    Caiu,
    Mudou,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum Condicao {
    /// Preço de um produto: pela página (`url`) ou, com a nuvem liberada, pelo nome (`produto`).
    Preco { #[serde(default)] url: String, #[serde(default)] produto: String, compara: Compara, #[serde(default)] valor: Option<f64> },
    /// Horário do dia entre `de` e `ate` ("05:00"–"12:00"; atravessa a meia-noite se `de` > `ate`).
    Janela { de: String, ate: String },
    DiaSemana { dias: Vec<u8> },
    /// Um programa aberto (ou fechado), pelo nome do processo ou da janela.
    ProgramaAberto { nome: String, #[serde(default = "sim")] aberto: bool },
    ArquivoExiste { caminho: String, #[serde(default = "sim")] existe: bool },
    /// Só se as ações ainda não rodaram hoje.
    UmaVezPorDia,
}

fn sim() -> bool { true }

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Condicoes {
    #[serde(default)]
    pub modo: Modo,
    #[serde(default)]
    pub itens: Vec<Condicao>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum Acao {
    /// Texto com {nome}, {preco}, {anterior}, {hora}.
    Notificar { #[serde(default)] titulo: String, texto: String },
    /// O modelo local escreve a mensagem na hora (com a agenda do dia); vai para a conversa
    /// "Rotinas" e para a notificação.
    MensagemAyla { pedido: String },
    Macro { id: String, #[serde(default)] nome: String },
    /// Comando no bash; só roda depois de aprovado na página (`aprovado` = o próprio comando).
    Comando { comando: String, #[serde(default)] aprovado: String },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Rotina {
    pub id: String,
    pub nome: String,
    #[serde(default)]
    pub ativa: bool,
    pub gatilho: Gatilho,
    #[serde(default)]
    pub condicoes: Condicoes,
    pub acoes: Vec<Acao>,
    /// Pode usar a nuvem (pesquisa na web) para checar preços que a página não mostra.
    #[serde(default)]
    pub nuvem: bool,
    /// Última vez que o gatilho disparou (segundos locais desde 1970).
    #[serde(default)]
    pub ultima: Option<i64>,
    /// Última vez que as ações rodaram.
    #[serde(default)]
    pub ultimo_disparo: Option<i64>,
    #[serde(default)]
    pub ultimo_preco: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Execucao {
    pub quando: i64,
    /// As condições valeram e as ações rodaram.
    pub disparou: bool,
    pub detalhe: String,
    #[serde(default)]
    pub preco: Option<f64>,
}

// ---------------------------------------------------------------------------------------------
// Guardar

/// Definições (na configuração, que viaja com os dotfiles) e estado do agendador (neste PC).
pub fn arquivo(config: &Path) -> PathBuf { config.join("rotinas.json") }
fn arquivo_estado(local: &Path) -> PathBuf { local.join("rotinas-estado.json") }
fn arquivo_historico(dir: &Path) -> PathBuf { dir.join("rotinas-historico.json") }

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
struct Estado { ultima: Option<i64>, ultimo_disparo: Option<i64>, ultimo_preco: Option<f64> }

/// Rotinas com o estado deste computador. Na primeira vez, traz o arquivo antigo (tudo junto,
/// nos dados da Ayla) para a configuração.
pub fn carregar(config: &Path, local: &Path) -> Vec<Rotina> {
    let antigo = local.join("rotinas.json");
    if !arquivo(config).exists() && antigo.exists() {
        let velhas: Vec<Rotina> = fs::read(&antigo).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        if guardar(config, local, &velhas).is_ok() { let _ = fs::rename(&antigo, antigo.with_extension("json.migrado")); }
    }
    let mut list: Vec<Rotina> = fs::read(arquivo(config)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let estados: HashMap<String, Estado> = fs::read(arquivo_estado(local)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    for r in list.iter_mut() {
        let e = estados.get(&r.id).cloned().unwrap_or_default();
        (r.ultima, r.ultimo_disparo, r.ultimo_preco) = (e.ultima, e.ultimo_disparo, e.ultimo_preco);
    }
    list
}

fn gravar(path: &Path, value: &impl Serialize) -> Result<(), String> {
    if let Some(d) = path.parent() { fs::create_dir_all(d).map_err(|_| "Não foi possível guardar as rotinas")?; }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(value).unwrap_or_default()).map_err(|_| "Não foi possível guardar as rotinas")?;
    fs::rename(&tmp, path).map_err(|_| "Não foi possível guardar as rotinas".into())
}

pub fn guardar(config: &Path, local: &Path, rotinas: &[Rotina]) -> Result<(), String> {
    let definicoes: Vec<Value> = rotinas.iter().map(|r| {
        let mut v = json!(r);
        if let Some(o) = v.as_object_mut() { for k in ["ultima", "ultimo_disparo", "ultimo_preco"] { o.remove(k); } }
        v
    }).collect();
    let estados: HashMap<&str, Estado> = rotinas.iter().map(|r| (r.id.as_str(), Estado { ultima: r.ultima, ultimo_disparo: r.ultimo_disparo, ultimo_preco: r.ultimo_preco })).collect();
    // A definição só é regravada se mudou (o agendador grava o estado a cada volta).
    let atual: Option<Vec<Value>> = fs::read(arquivo(config)).ok().and_then(|b| serde_json::from_slice(&b).ok());
    if atual.as_ref() != Some(&definicoes) { gravar(&arquivo(config), &definicoes)?; }
    gravar(&arquivo_estado(local), &estados)
}

pub fn historico(dir: &Path) -> HashMap<String, Vec<Execucao>> {
    fs::read(arquivo_historico(dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Acrescenta uma execução (até 300 por rotina).
pub fn registrar(dir: &Path, id: &str, execucao: Execucao) -> Result<(), String> {
    let mut all = historico(dir);
    let list = all.entry(id.to_string()).or_default();
    list.push(execucao);
    if list.len() > 300 { let excess = list.len() - 300; list.drain(..excess); }
    gravar(&arquivo_historico(dir), &all)
}

pub fn esquecer_historico(dir: &Path, id: &str) -> Result<(), String> {
    let mut all = historico(dir);
    if all.remove(id).is_some() { gravar(&arquivo_historico(dir), &all)?; }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Conferir

fn hora(text: &str) -> Option<NaiveTime> { NaiveTime::parse_from_str(text.trim(), "%H:%M").ok() }

pub fn validar(r: &Rotina) -> Result<(), String> {
    if r.nome.trim().is_empty() { return Err("Dê um nome à rotina".into()); }
    if r.id.is_empty() || !r.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') { return Err("Rotina inválida".into()); }
    if r.acoes.is_empty() { return Err("A rotina precisa de pelo menos uma ação".into()); }
    let tem_preco = r.condicoes.itens.iter().any(|c| matches!(c, Condicao::Preco { .. }));
    match &r.gatilho {
        Gatilho::Intervalo { minutos } if *minutos < 1 || *minutos > 7 * 24 * 60 => return Err("O intervalo vai de 1 minuto a 7 dias".into()),
        Gatilho::Intervalo { minutos } if tem_preco && *minutos < 15 => return Err("Para checar preço, use um intervalo de 15 minutos ou mais (as lojas bloqueiam acessos seguidos)".into()),
        Gatilho::Horario { horas, dias } => {
            if horas.is_empty() || horas.iter().any(|h| hora(h).is_none()) { return Err("Use horários como 08:00".into()); }
            if dias.iter().any(|d| *d > 6) { return Err("Dia da semana inválido".into()); }
        }
        _ => {}
    }
    for c in &r.condicoes.itens {
        match c {
            Condicao::Preco { url, produto, compara, valor } => {
                if url.trim().is_empty() && produto.trim().is_empty() { return Err("Diga o link da loja (ou o produto, com a nuvem liberada)".into()); }
                if !url.trim().is_empty() && !(url.starts_with("https://") || url.starts_with("http://")) { return Err("O link da loja precisa começar com https://".into()); }
                if url.trim().is_empty() && !r.nuvem { return Err("Sem o link da loja, a checagem precisa da nuvem: cole o link ou libere a nuvem nesta rotina".into()); }
                if matches!(compara, Compara::Abaixo | Compara::Acima) && valor.is_none_or(|v| v <= 0.0) { return Err("Diga o valor do preço para comparar".into()); }
            }
            Condicao::Janela { de, ate } if hora(de).is_none() || hora(ate).is_none() => return Err("Use horários como 05:00".into()),
            Condicao::DiaSemana { dias } if dias.is_empty() || dias.iter().any(|d| *d > 6) => return Err("Escolha os dias da semana".into()),
            Condicao::ProgramaAberto { nome, .. } if nome.trim().is_empty() => return Err("Diga o nome do programa".into()),
            Condicao::ArquivoExiste { caminho, .. } if !Path::new(caminho).is_absolute() => return Err("Use o caminho completo do arquivo".into()),
            _ => {}
        }
    }
    for a in &r.acoes {
        match a {
            Acao::Notificar { texto, .. } if texto.trim().is_empty() => return Err("Escreva o texto da notificação".into()),
            Acao::MensagemAyla { pedido } if pedido.trim().is_empty() => return Err("Diga o que a Ayla deve escrever".into()),
            Acao::Macro { id, .. } if id.is_empty() => return Err("Escolha a macro".into()),
            Acao::Comando { comando, .. } if comando.trim().is_empty() || comando.contains('\n') => return Err("Escreva o comando numa linha".into()),
            _ => {}
        }
    }
    Ok(())
}

fn segundos(t: NaiveDateTime) -> i64 { t.and_utc().timestamp() }
fn data(s: i64) -> Option<NaiveDateTime> { chrono::DateTime::from_timestamp(s, 0).map(|d| d.naive_utc()) }
fn dia_semana(t: NaiveDateTime) -> u8 { t.weekday().num_days_from_monday() as u8 }

/// O gatilho dispara agora? `entrou`: primeira volta depois de entrar na sessão.
pub fn vencida(r: &Rotina, agora: NaiveDateTime, entrou: bool) -> bool {
    if !r.ativa { return false; }
    let now = segundos(agora);
    match &r.gatilho {
        Gatilho::Intervalo { minutos } => r.ultima.is_none_or(|u| now - u >= i64::from(*minutos) * 60),
        Gatilho::AoEntrar => entrou,
        Gatilho::Horario { horas, dias } => {
            if !dias.is_empty() && !dias.contains(&dia_semana(agora)) { return false; }
            horas.iter().filter_map(|h| hora(h)).any(|h| {
                let quando = segundos(agora.date().and_time(h));
                // Perdeu por pouco (PC dormindo): até 2 h depois ainda vale.
                quando <= now && now - quando < 2 * 3600 && r.ultima.is_none_or(|u| u < quando)
            })
        }
    }
}

/// Próxima vez que o gatilho dispara (para mostrar), em segundos locais.
pub fn proxima(r: &Rotina, agora: NaiveDateTime) -> Option<i64> {
    if !r.ativa { return None; }
    match &r.gatilho {
        Gatilho::Intervalo { minutos } => Some(r.ultima.map_or(segundos(agora), |u| u + i64::from(*minutos) * 60)),
        Gatilho::AoEntrar => None,
        Gatilho::Horario { horas, dias } => (0..8).flat_map(|d| {
            let dia = agora.date() + chrono::Duration::days(d);
            horas.iter().filter_map(|h| hora(h)).map(move |h| dia.and_time(h)).collect::<Vec<_>>()
        }).filter(|t| *t > agora && (dias.is_empty() || dias.contains(&dia_semana(*t)))).map(segundos).min(),
    }
}

fn mesmo_dia(a: i64, b: NaiveDate) -> bool { data(a).is_some_and(|d| d.date() == b) }

/// Condições que não dependem de rede. `None` para as de preço (conferidas à parte).
pub fn condicao_local(c: &Condicao, r: &Rotina, agora: NaiveDateTime, programa_aberto: &dyn Fn(&str) -> bool) -> Option<bool> {
    Some(match c {
        Condicao::Preco { .. } => return None,
        Condicao::Janela { de, ate } => {
            let (Some(de), Some(ate)) = (hora(de), hora(ate)) else { return Some(false) };
            let t = agora.time().with_second(0).unwrap_or(agora.time());
            if de <= ate { t >= de && t <= ate } else { t >= de || t <= ate }
        }
        Condicao::DiaSemana { dias } => dias.contains(&dia_semana(agora)),
        Condicao::ProgramaAberto { nome, aberto } => programa_aberto(nome) == *aberto,
        Condicao::ArquivoExiste { caminho, existe } => Path::new(caminho).exists() == *existe,
        Condicao::UmaVezPorDia => !r.ultimo_disparo.is_some_and(|u| mesmo_dia(u, agora.date())),
    })
}

/// Preço contra a regra (com o preço anterior para "caiu"/"mudou").
pub fn compara_preco(compara: Compara, valor: Option<f64>, preco: f64, anterior: Option<f64>) -> bool {
    match compara {
        Compara::Sempre => true,
        Compara::Abaixo => valor.is_some_and(|v| preco < v),
        Compara::Acima => valor.is_some_and(|v| preco > v),
        Compara::Caiu => anterior.is_some_and(|a| preco < a - 0.004),
        Compara::Mudou => anterior.is_some_and(|a| (preco - a).abs() > 0.004),
    }
}

/// Junta os resultados conforme o modo (sem condições: vale).
pub fn juntar(modo: Modo, resultados: &[bool]) -> bool {
    if resultados.is_empty() { return true; }
    match modo { Modo::Todas => resultados.iter().all(|r| *r), Modo::Qualquer => resultados.iter().any(|r| *r) }
}

// ---------------------------------------------------------------------------------------------
// Preço numa página

/// "1.899,90" / "1899.90" / "1,899.90" → 1899.9.
pub fn numero(text: &str) -> Option<f64> {
    let t: String = text.trim().chars().filter(|c| c.is_ascii_digit() || *c == ',' || *c == '.').collect();
    if t.is_empty() { return None; }
    let (last_comma, last_dot) = (t.rfind(','), t.rfind('.'));
    let normal = match (last_comma, last_dot) {
        (Some(c), Some(d)) if c > d => t.replace('.', "").replace(',', "."),
        (Some(_), Some(_)) => t.replace(',', ""),
        (Some(c), None) if t.len() - c - 1 == 2 || t.len() - c - 1 == 1 => t.replace(',', "."),
        (Some(_), None) => t.replace(',', ""),
        // Só pontos: vários, ou três dígitos depois do único, são milhar ("12.999").
        (None, Some(d)) if t.matches('.').count() > 1 || t.len() - d - 1 == 3 => t.replace('.', ""),
        _ => t,
    };
    normal.parse::<f64>().ok().filter(|v| *v > 0.0 && *v < 10_000_000.0)
}

/// Lê o preço do HTML: dados estruturados (JSON-LD, `itemprop`, `product:price`) e, se não houver,
/// o primeiro "R$ …" da página.
pub fn preco_do_html(html: &str) -> Option<f64> {
    let lower = html.to_lowercase();
    // Metadados: <meta property="product:price:amount" content="1899.90">, itemprop="price".
    for key in ["product:price:amount", "og:price:amount", "itemprop=\"price\"", "itemprop='price'"] {
        let mut from = 0;
        while let Some(at) = lower[from..].find(key) {
            let start = from + at;
            let tag_start = lower[..start].rfind('<').unwrap_or(start);
            let tag_end = lower[start..].find('>').map_or(lower.len(), |e| start + e);
            let tag = &html[tag_start..tag_end];
            if let Some(c) = tag.to_lowercase().find("content=") {
                let rest = &tag[c + 8..];
                let quote = rest.chars().next().unwrap_or('"');
                let value = rest.trim_start_matches(['"', '\'']).split(quote).next().unwrap_or("");
                if let Some(v) = numero(value) { return Some(v); }
            }
            from = tag_end.max(start + 1);
        }
    }
    // JSON-LD: "price": "1899.90" ou "lowPrice": 1899.9 (dentro de "offers").
    for key in ["\"price\"", "\"lowprice\""] {
        let mut from = 0;
        while let Some(at) = lower[from..].find(key) {
            let start = from + at + key.len();
            let rest = html[start..].trim_start().trim_start_matches(':').trim_start().trim_start_matches('"');
            let value: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',').collect();
            if let Some(v) = numero(&value) { return Some(v); }
            from = start;
        }
    }
    preco_do_texto(&texto_visivel(html, 200_000))
}

/// "R$ …" no texto: o de "por R$ …"/"… à vista" primeiro; fora o riscado ("De R$ …") e parcelas
/// ("10x de R$ …"); senão, o primeiro.
pub fn preco_do_texto(texto: &str) -> Option<f64> {
    let lower = texto.to_lowercase();
    let mut primeiro = None;
    let mut from = 0;
    while let Some(at) = texto[from..].find("R$") {
        let pos = from + at;
        let start = pos + 2;
        from = start;
        let value: String = texto[start..].trim_start().chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',').collect();
        let Some(v) = numero(&value) else { continue };
        let antes: String = lower[..pos].chars().rev().take(14).collect::<Vec<_>>().into_iter().rev().collect();
        let antes = antes.trim_end();
        let depois: String = lower[start..].chars().take(value.len() + 16).collect();
        if antes.ends_with(" de") || antes == "de" || antes.ends_with("x de") || antes.ends_with("x") { continue; }
        if antes.ends_with("por") || depois.contains("vista") { return Some(v); }
        primeiro.get_or_insert(v);
    }
    primeiro
}

/// Texto da página sem tags, scripts e estilos (para o modelo procurar o preço).
pub fn texto_visivel(html: &str, limite: usize) -> String {
    let mut out = String::new();
    let lower = html.to_lowercase();
    let mut i = 0;
    let bytes = html.as_bytes();
    while i < html.len() && out.len() < limite {
        if bytes[i] == b'<' {
            let skip_to = ["script", "style", "noscript", "svg"].iter().find(|t| lower[i + 1..].starts_with(*t))
                .and_then(|t| lower[i..].find(&format!("</{t}")).map(|e| i + e));
            let from = skip_to.unwrap_or(i);
            i = lower[from..].find('>').map_or(html.len(), |e| from + e + 1);
            out.push(' ');
            continue;
        }
        let next = lower[i..].find('<').map_or(html.len(), |e| i + e);
        out.push_str(&html[i..next]);
        i = next;
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ").replace("&nbsp;", " ")
}

pub fn preco_pedido(texto: &str) -> Vec<Value> {
    vec![
        json!({"role":"system","content":"Você lê o texto de uma página de produto e diz o preço atual à vista, em reais. Responda só o JSON {\"preco\": número} (ou null se a página não mostrar preço). Ignore parcelas, frete e preços riscados (\"de\")."}),
        json!({"role":"user","content": texto}),
    ]
}

pub fn preco_schema() -> Value { json!({"type":"object","properties":{"preco":{"type":["number","null"]}},"required":["preco"]}) }

pub fn ler_preco_modelo(raw: &str) -> Option<f64> {
    let start = raw.find('{')?;
    let v: Value = serde_json::from_str(&raw[start..raw.rfind('}')? + 1]).ok()?;
    v["preco"].as_f64().filter(|p| *p > 0.0).or_else(|| v["preco"].as_str().and_then(numero))
}

/// Preço na resposta da pesquisa da nuvem (o primeiro "R$ …" ou um número JSON).
pub fn preco_da_resposta(text: &str) -> Option<f64> {
    ler_preco_modelo(text).or_else(|| {
        let at = text.find("R$")?;
        numero(&text[at + 2..].trim_start().chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',').collect::<String>())
    })
}

/// Troca {nome}, {preco}, {anterior} e {hora} no texto de uma notificação.
pub fn preencher(texto: &str, nome: &str, preco: Option<f64>, anterior: Option<f64>, agora: NaiveDateTime) -> String {
    let reais = |v: Option<f64>| v.map(|v| format!("R$ {}", formatar(v))).unwrap_or_else(|| "—".into());
    // "R$ {preco}" (como o modelo às vezes escreve) não vira "R$ R$ …".
    let texto = texto.replace("R$ {preco}", "{preco}").replace("R${preco}", "{preco}").replace("R$ {anterior}", "{anterior}").replace("R${anterior}", "{anterior}");
    texto.replace("{nome}", nome).replace("{preco}", &reais(preco)).replace("{anterior}", &reais(anterior)).replace("{hora}", &agora.format("%H:%M").to_string())
}

/// 1899.9 → "1.899,90".
pub fn formatar(v: f64) -> String {
    let cents = (v * 100.0).round() as i64;
    let (int, frac) = (cents / 100, cents % 100);
    let digits = int.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 { out.push('.'); }
        out.push(c);
    }
    format!("{out},{frac:02}")
}

// ---------------------------------------------------------------------------------------------
// Pela conversa

/// Pedido de rotina na conversa: repetição ("a cada 1 hora", "sempre que eu abrir o PC") com algo
/// para fazer, ou acompanhar um preço. Lembretes simples ("me lembra amanhã às 9") ficam com os lembretes.
pub fn pedido(message: &str) -> bool {
    let t = format!(" {} ", super::hand::fold(message).replace(['?', '!', ',', '.'], " "));
    let has = |xs: &[&str]| xs.iter().any(|x| t.contains(x));
    let acao = has(&[" checa", " chequ", " verifica", " monitora", " confere", " acompanha", " olha ", " avisa", " avise", " notifica", " notificacao", " manda", " mande", " envia", " roda ", " executa", " me lembra", " me lembre", " bom dia", " mensagem"]);
    let intervalo = (t.contains(" a cada ") && has(&[" minuto", " hora", " dia", " semana"])) || has(&[" de hora em hora", " de meia em meia"]);
    let sessao = has(&[" sempre que eu abrir", " sempre que abrir", " quando eu abrir o", " quando eu ligar o", " sempre que ligar", " sempre que eu ligar", " ao ligar o pc", " ao abrir o pc", " ao entrar no pc", " quando eu entrar", " toda vez que eu ligar", " toda vez que eu abrir", " toda vez que ligar", " toda vez que abrir"]);
    let preco = has(&[" preco", " valor do ", " valor da "]) && has(&[" cair", " baixar", " abaixo", " menor", " mudar", " a cada", " todo dia", " monitora", " acompanha", " checa", " verifica"]);
    let explicita = has(&[" rotina", " schedule", " automacao", " agendamento"]) && has(&[" cria", " crie", " criar", " faz ", " faca", " quero", " configura"]);
    (acao && (intervalo || sessao)) || preco || explicita
}

pub fn pedido_modelo(message: &str, agora: NaiveDateTime) -> Vec<Value> {
    vec![
        json!({"role":"system","content": format!("Transforme o pedido em uma rotina da Ayla (agora: {}). JSON com: nome (curto); gatilho: {{\"tipo\":\"intervalo\",\"minutos\":N}} | {{\"tipo\":\"horario\",\"horas\":[\"HH:MM\"],\"dias\":[0-6, 0=segunda; vazio=todos]}} | {{\"tipo\":\"ao_entrar\"}} (ao ligar/abrir o PC); condicoes: {{\"modo\":\"todas\"|\"qualquer\",\"itens\":[...]}} com itens {{\"tipo\":\"preco\",\"url\":\"link se houver\",\"produto\":\"nome\",\"compara\":\"sempre\"|\"abaixo\"|\"acima\"|\"caiu\"|\"mudou\",\"valor\":número ou null}}, {{\"tipo\":\"janela\",\"de\":\"HH:MM\",\"ate\":\"HH:MM\"}} (\"de manhã\" = 05:00 a 12:00, \"à tarde\" = 12:00 a 18:00, \"à noite\" = 18:00 a 23:59), {{\"tipo\":\"dia_semana\",\"dias\":[...]}}, {{\"tipo\":\"programa_aberto\",\"nome\":\"...\",\"aberto\":true}}, {{\"tipo\":\"uma_vez_por_dia\"}}; acoes: [{{\"tipo\":\"notificar\",\"titulo\":\"...\",\"texto\":\"... pode usar {{preco}}, {{anterior}}, {{nome}}\"}} | {{\"tipo\":\"mensagem_ayla\",\"pedido\":\"o que a Ayla deve escrever\"}}]. Checar preço sem dizer quando avisar = compara \"sempre\" e notificar com o preço. \"Ao abrir o PC de manhã\" = ao_entrar + janela 05:00–12:00 + uma_vez_por_dia. Não invente links. Responda só o JSON.", agora.format("%d/%m/%Y %H:%M (%A)"))}),
        json!({"role":"user","content": message}),
    ]
}

pub fn pedido_schema() -> Value {
    json!({"type":"object","properties":{"nome":{"type":"string"},"gatilho":{"type":"object"},"condicoes":{"type":"object"},"acoes":{"type":"array"}},"required":["nome","gatilho","acoes"]})
}

/// Rotina a partir da resposta do modelo (inativa até a pessoa ativar).
pub fn ler_pedido(raw: &str, id: String) -> Option<Rotina> {
    let start = raw.find('{')?;
    let mut v: Value = serde_json::from_str(&raw[start..raw.rfind('}')? + 1]).ok()?;
    if v["condicoes"].is_null() { v["condicoes"] = json!({"modo":"todas","itens":[]}); }
    // Nulos que o modelo manda nos opcionais.
    if let Some(items) = v["condicoes"]["itens"].as_array_mut() {
        for it in items.iter_mut() { if let Some(o) = it.as_object_mut() { o.retain(|_, val| !val.is_null()); } }
    }
    v["id"] = json!(id);
    v["ativa"] = json!(false);
    serde_json::from_value::<Rotina>(v).ok().filter(|r| !r.acoes.is_empty())
}

/// Frase curta que descreve a rotina ("A cada 60 min · se o preço cair · notificação").
pub fn resumo(r: &Rotina) -> String {
    let quando = match &r.gatilho {
        Gatilho::Intervalo { minutos } if minutos % 60 == 0 => format!("A cada {} h", minutos / 60),
        Gatilho::Intervalo { minutos } => format!("A cada {minutos} min"),
        Gatilho::Horario { horas, .. } => format!("Às {}", horas.join(", ")),
        Gatilho::AoEntrar => "Ao entrar na sessão".into(),
    };
    let se: Vec<String> = r.condicoes.itens.iter().map(|c| match c {
        Condicao::Preco { compara: Compara::Sempre, .. } => "checa o preço".into(),
        Condicao::Preco { compara: Compara::Abaixo, valor, .. } => format!("preço abaixo de R$ {}", formatar(valor.unwrap_or(0.0))),
        Condicao::Preco { compara: Compara::Acima, valor, .. } => format!("preço acima de R$ {}", formatar(valor.unwrap_or(0.0))),
        Condicao::Preco { compara: Compara::Caiu, .. } => "se o preço cair".into(),
        Condicao::Preco { compara: Compara::Mudou, .. } => "se o preço mudar".into(),
        Condicao::Janela { de, ate } => format!("entre {de} e {ate}"),
        Condicao::DiaSemana { .. } => "em certos dias".into(),
        Condicao::ProgramaAberto { nome, aberto } => format!("{nome} {}", if *aberto { "aberto" } else { "fechado" }),
        Condicao::ArquivoExiste { .. } => "se o arquivo existir".into(),
        Condicao::UmaVezPorDia => "uma vez por dia".into(),
    }).collect();
    let faz: Vec<&str> = r.acoes.iter().map(|a| match a { Acao::Notificar { .. } => "notificação", Acao::MensagemAyla { .. } => "mensagem da Ayla", Acao::Macro { .. } => "macro", Acao::Comando { .. } => "comando" }).collect();
    let juntar_se = if r.condicoes.modo == Modo::Qualquer { " ou " } else { ", " };
    [quando, se.join(juntar_se), faz.join(" + ")].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(d: u32, h: u32, m: u32) -> NaiveDateTime { NaiveDate::from_ymd_opt(2026, 11, d).unwrap().and_hms_opt(h, m, 0).unwrap() }
    fn rotina(gatilho: Gatilho) -> Rotina {
        Rotina { id: "r-1".into(), nome: "Teste".into(), ativa: true, gatilho, condicoes: Condicoes::default(), acoes: vec![Acao::Notificar { titulo: String::new(), texto: "oi".into() }], nuvem: false, ultima: None, ultimo_disparo: None, ultimo_preco: None }
    }

    #[test]
    fn gatilhos_disparam_na_hora_certa() {
        let mut r = rotina(Gatilho::Intervalo { minutos: 60 });
        assert!(vencida(&r, at(27, 10, 0), false), "primeira vez roda");
        r.ultima = Some(segundos(at(27, 10, 0)));
        assert!(!vencida(&r, at(27, 10, 59), false));
        assert!(vencida(&r, at(27, 11, 0), false));
        r.ativa = false;
        assert!(!vencida(&r, at(27, 12, 0), false), "pausada");
        // 27/11/2026 é sexta (4).
        let mut h = rotina(Gatilho::Horario { horas: vec!["08:00".into()], dias: vec![0, 1, 2, 3, 4] });
        assert!(!vencida(&h, at(27, 7, 59), false));
        assert!(vencida(&h, at(27, 8, 1), false));
        assert!(!vencida(&h, at(28, 8, 1), false), "sábado fora dos dias");
        assert!(!vencida(&h, at(27, 10, 30), false), "passou de 2 h: perdeu");
        h.ultima = Some(segundos(at(27, 8, 1)));
        assert!(!vencida(&h, at(27, 8, 30), false), "já rodou hoje");
        assert_eq!(proxima(&h, at(27, 9, 0)), Some(segundos(at(30, 8, 0))), "próxima: segunda");
        let e = rotina(Gatilho::AoEntrar);
        assert!(vencida(&e, at(27, 8, 0), true) && !vencida(&e, at(27, 8, 0), false));
    }

    #[test]
    fn condicoes_combinadas() {
        let mut r = rotina(Gatilho::AoEntrar);
        let nada = |_: &str| false;
        let manha = Condicao::Janela { de: "05:00".into(), ate: "12:00".into() };
        assert_eq!(condicao_local(&manha, &r, at(27, 8, 0), &nada), Some(true));
        assert_eq!(condicao_local(&manha, &r, at(27, 14, 0), &nada), Some(false));
        let noite = Condicao::Janela { de: "22:00".into(), ate: "02:00".into() };
        assert_eq!(condicao_local(&noite, &r, at(27, 1, 0), &nada), Some(true), "atravessa a meia-noite");
        assert_eq!(condicao_local(&Condicao::UmaVezPorDia, &r, at(27, 8, 0), &nada), Some(true));
        r.ultimo_disparo = Some(segundos(at(27, 7, 0)));
        assert_eq!(condicao_local(&Condicao::UmaVezPorDia, &r, at(27, 9, 0), &nada), Some(false));
        assert_eq!(condicao_local(&Condicao::UmaVezPorDia, &r, at(28, 7, 0), &nada), Some(true), "outro dia");
        let steam = |n: &str| n == "steam";
        assert_eq!(condicao_local(&Condicao::ProgramaAberto { nome: "steam".into(), aberto: false }, &r, at(27, 8, 0), &steam), Some(false));
        assert!(juntar(Modo::Todas, &[true, true]) && !juntar(Modo::Todas, &[true, false]));
        assert!(juntar(Modo::Qualquer, &[false, true]) && !juntar(Modo::Qualquer, &[false, false]));
        assert!(juntar(Modo::Todas, &[]));
        assert!(compara_preco(Compara::Abaixo, Some(2000.0), 1899.9, None));
        assert!(!compara_preco(Compara::Caiu, None, 1899.9, None), "sem anterior não caiu");
        assert!(compara_preco(Compara::Caiu, None, 1899.9, Some(1999.0)));
        assert!(!compara_preco(Compara::Mudou, None, 10.0, Some(10.0)));
    }

    #[test]
    fn preco_nas_paginas() {
        assert_eq!(numero("1.899,90"), Some(1899.9));
        assert_eq!(numero("1899.90"), Some(1899.9));
        assert_eq!(numero("1,899.90"), Some(1899.9));
        assert_eq!(numero("12.999"), Some(12999.0));
        assert_eq!(numero("49,9"), Some(49.9));
        assert_eq!(preco_do_html(r#"<meta property="product:price:amount" content="2499.00"><span>R$ 9,99</span>"#), Some(2499.0));
        assert_eq!(preco_do_html(r#"<script type="application/ld+json">{"@type":"Product","offers":{"@type":"Offer","price":"1899.90","priceCurrency":"BRL"}}</script>"#), Some(1899.9));
        assert_eq!(preco_do_html(r#"<span itemprop="price" content="349.9">R$ 349,90</span>"#), Some(349.9));
        assert_eq!(preco_do_html("<script>var x = 'R$ 1,00';</script><div>Por <b>R$&nbsp;</b> R$ 3.299,00 à vista</div>"), Some(3299.0));
        assert_eq!(preco_do_html("<p>sem preço</p>"), None);
        assert_eq!(preco_do_texto("De R$ 2.000,00 por R$ 1.499,00 à vista ou 10x de R$ 149,90"), Some(1499.0));
        assert_eq!(preco_do_texto("12x de R$ 99,90 sem juros. R$ 1.198,80"), Some(1198.8));
        assert_eq!(preco_do_texto("Preço: R$ 59,90"), Some(59.9));
        assert_eq!(texto_visivel("<style>a{}</style><p>Olá <b>mundo</b></p><script>x()</script>", 100), "Olá mundo");
        assert_eq!(ler_preco_modelo("{\"preco\": 1299.5}"), Some(1299.5));
        assert_eq!(ler_preco_modelo("{\"preco\": null}"), None);
        assert_eq!(preco_da_resposta("O preço atual é R$ 4.199,00 na loja X."), Some(4199.0));
        assert_eq!(formatar(1899.9), "1.899,90");
        assert_eq!(preencher("{nome}: {preco} (antes {anterior})", "RTX", Some(1899.9), None, at(27, 9, 5)), "RTX: R$ 1.899,90 (antes —)");
        assert_eq!(preencher("Abaixo de R$ {preco}!", "x", Some(10.0), None, at(27, 9, 5)), "Abaixo de R$ 10,00!");
    }

    #[test]
    fn definicoes_na_configuracao_e_estado_neste_pc() {
        let root = std::env::temp_dir().join(format!("ayla-rotinas-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (config, local) = (root.join("config"), root.join("local"));
        fs::create_dir_all(&local).unwrap();
        let mut r = rotina(Gatilho::Intervalo { minutos: 60 });
        r.ultima = Some(123); r.ultimo_preco = Some(10.0);
        // Arquivo antigo (tudo junto nos dados da Ayla): migra.
        fs::write(local.join("rotinas.json"), serde_json::to_vec(&vec![r.clone()]).unwrap()).unwrap();
        let list = carregar(&config, &local);
        assert_eq!(list, vec![r.clone()]);
        let def = fs::read_to_string(arquivo(&config)).unwrap();
        assert!(!def.contains("ultima") && !def.contains("ultimo_preco"), "estado não vai para os dotfiles: {def}");
        assert!(local.join("rotinas.json.migrado").exists());
        r.ultima = Some(999);
        let antes = fs::metadata(arquivo(&config)).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        guardar(&config, &local, &[r.clone()]).unwrap();
        assert_eq!(fs::metadata(arquivo(&config)).unwrap().modified().unwrap(), antes, "só o estado mudou: a definição não é regravada");
        assert_eq!(carregar(&config, &local)[0].ultima, Some(999));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn pedidos_na_conversa_e_validacao() {
        for m in ["checa o preço desse monitor a cada 1 hora https://loja.com/x", "me manda uma notificação de bom dia sempre que eu abrir o pc de manhã", "me avisa quando o preço da RTX 5070 cair", "cria uma rotina que verifica meu e-mail", "me lembra de beber água a cada 2 horas"] {
            assert!(pedido(m), "{m}");
        }
        for m in ["me lembra amanhã às 9 de ligar pro médico", "qual o preço da RTX 5070?", "organiza meus downloads", "a cada dia que passa fico mais cansado"] {
            assert!(!pedido(m), "{m}");
        }
        let raw = r#"{"nome":"Preço do monitor","gatilho":{"tipo":"intervalo","minutos":60},"condicoes":{"modo":"todas","itens":[{"tipo":"preco","url":"https://loja.com/x","produto":"monitor","compara":"sempre","valor":null}]},"acoes":[{"tipo":"notificar","titulo":"Monitor","texto":"Agora: {preco}"}]}"#;
        let r = ler_pedido(raw, "r-2".into()).unwrap();
        assert!(!r.ativa && validar(&r).is_ok());
        assert_eq!(resumo(&r), "A cada 1 h · checa o preço · notificação");
        let mut sem_link = r.clone();
        sem_link.condicoes.itens = vec![Condicao::Preco { url: String::new(), produto: "RTX".into(), compara: Compara::Sempre, valor: None }];
        assert!(validar(&sem_link).unwrap_err().contains("nuvem"));
        sem_link.nuvem = true;
        assert!(validar(&sem_link).is_ok());
        let mut rapido = r.clone();
        rapido.gatilho = Gatilho::Intervalo { minutos: 5 };
        assert!(validar(&rapido).is_err(), "preço a cada 5 min: não");
        let bom_dia = r#"{"nome":"Bom dia","gatilho":{"tipo":"ao_entrar"},"condicoes":{"modo":"todas","itens":[{"tipo":"janela","de":"05:00","ate":"12:00"},{"tipo":"uma_vez_por_dia"}]},"acoes":[{"tipo":"mensagem_ayla","pedido":"Bom dia com a agenda de hoje"}]}"#;
        let b = ler_pedido(bom_dia, "r-3".into()).unwrap();
        assert_eq!(resumo(&b), "Ao entrar na sessão · entre 05:00 e 12:00, uma vez por dia · mensagem da Ayla");
    }
}
