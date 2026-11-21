//! Lembretes: "me lembra às 15h de ligar pro médico".
//!
//! Regras fixas entendem as formas comuns (hora, hoje/amanhã, dia da semana, "dia 25", "daqui a
//! 20 minutos", repetições simples). O que elas não entendem vai para o modelo local, com a data
//! e a hora atuais. A Ayla sempre confirma data e hora na resposta. Horários são locais.
use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Weekday};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Repeat {
    None,
    Daily,
    Weekly,
    Monthly,
}
impl Repeat {
    pub fn as_str(self) -> &'static str {
        match self {
            Repeat::None => "none",
            Repeat::Daily => "daily",
            Repeat::Weekly => "weekly",
            Repeat::Monthly => "monthly",
        }
    }
    pub fn parse(value: &str) -> Self {
        match value {
            "daily" => Repeat::Daily,
            "weekly" => Repeat::Weekly,
            "monthly" => Repeat::Monthly,
            _ => Repeat::None,
        }
    }
}

/// Lembrete entendido a partir do pedido.
#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    pub when: NaiveDateTime,
    pub repeat: Repeat,
    pub text: String,
}

const TRIGGERS: &[&str] = &[
    "me lembra", "me lembre", "lembre-me", "lembra-me", "me lembrar", "me avisa", "me avise", "cria um lembrete",
    "crie um lembrete", "criar lembrete", "novo lembrete", "lembrete para", "lembrete pra", "lembrete de",
];

/// Pedido de lembrete ("me lembra…", "me avisa…", "cria um lembrete…").
pub fn is_request(message: &str) -> bool {
    let text = message.to_lowercase();
    TRIGGERS.iter().any(|t| text.contains(t))
}

/// Palavras que levam o pedido ao caminho local (a interface usa a mesma lista).
pub const WORDS: &[&str] = TRIGGERS;

fn number(word: &str) -> Option<u32> {
    word.parse().ok().or(match word {
        "um" | "uma" => Some(1), "dois" | "duas" => Some(2), "três" | "tres" => Some(3), "quatro" => Some(4),
        "cinco" => Some(5), "seis" => Some(6), "sete" => Some(7), "oito" => Some(8), "nove" => Some(9),
        "dez" => Some(10), "onze" => Some(11), "doze" => Some(12), "quinze" => Some(15), "vinte" => Some(20),
        "trinta" => Some(30), "meia" => Some(30), _ => None,
    })
}

fn weekday(word: &str) -> Option<Weekday> {
    Some(match word.trim_end_matches("-feira").trim_end_matches('s') {
        "segunda" => Weekday::Mon,
        "terça" | "terca" => Weekday::Tue,
        "quarta" => Weekday::Wed,
        "quinta" => Weekday::Thu,
        "sexta" => Weekday::Fri,
        "sábado" | "sabado" => Weekday::Sat,
        "domingo" => Weekday::Sun,
        _ => return None,
    })
}

/// Hora citada: "15h", "15h30", "15:30", "às 9", "9 da manhã", "3 da tarde", "meio-dia".
fn time_of(words: &[String]) -> Option<(NaiveTime, Vec<usize>)> {
    let joined = words.join(" ");
    if joined.contains("meio-dia") || joined.contains("meio dia") {
        let at = words.iter().position(|w| w.starts_with("meio")).unwrap_or(0);
        let mut used = vec![at];
        if words.get(at + 1).is_some_and(|w| w == "dia") {
            used.push(at + 1);
        }
        return Some((NaiveTime::from_hms_opt(12, 0, 0)?, used));
    }
    if joined.contains("meia-noite") || joined.contains("meia noite") {
        let at = words.iter().position(|w| w.starts_with("meia")).unwrap_or(0);
        return Some((NaiveTime::from_hms_opt(23, 59, 0)?, vec![at]));
    }
    for (i, word) in words.iter().enumerate() {
        let after_at = i > 0 && matches!(words[i - 1].as_str(), "às" | "as" | "às," | "a");
        let (hour, minute) = if let Some((h, m)) = word.split_once(':') {
            (h.parse::<u32>().ok(), m.trim_end_matches('h').parse::<u32>().ok())
        } else if let Some((h, m)) = word.split_once('h') {
            (h.parse::<u32>().ok(), if m.is_empty() { Some(0) } else { m.parse::<u32>().ok() })
        } else if after_at || words.get(i + 1).is_some_and(|n| n == "da" || n == "horas") {
            (number(word), Some(0))
        } else {
            (None, None)
        };
        let (Some(mut hour), Some(minute)) = (hour, minute) else { continue };
        let mut used = vec![i];
        if i > 0 && matches!(words[i - 1].as_str(), "às" | "as") {
            used.push(i - 1);
        }
        if words.get(i + 1).is_some_and(|w| w == "horas") {
            used.push(i + 1);
        }
        if words.get(i + 1).is_some_and(|w| w == "da") {
            let part = words.get(i + 2).map(String::as_str);
            if matches!(part, Some("tarde" | "noite")) && hour < 12 {
                hour += 12;
            }
            if matches!(part, Some("tarde" | "noite" | "manhã" | "manha" | "madrugada")) {
                used.extend([i + 1, i + 2]);
            }
        }
        if let Some(time) = NaiveTime::from_hms_opt(hour, minute, 0) {
            return Some((time, used));
        }
    }
    None
}

fn words_of(message: &str) -> Vec<String> {
    message
        .to_lowercase()
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | '.' | '!' | '?' | ';'))
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

/// Interpreta o pedido em relação a `now` (horário local). `None` quando as regras não entendem.
pub fn parse(message: &str, now: NaiveDateTime) -> Option<Parsed> {
    let words = words_of(message);
    let mut used: Vec<usize> = Vec::new();
    let mut repeat = Repeat::None;
    let mut day: Option<NaiveDate> = None;
    let mut weekly_on: Option<Weekday> = None;
    let mut when: Option<NaiveDateTime> = None;

    for (i, word) in words.iter().enumerate() {
        let next = words.get(i + 1).map(String::as_str);
        // Repetições: "todo dia", "todos os dias", "toda segunda", "toda semana", "todo mês".
        if matches!(word.as_str(), "todo" | "toda" | "todos" | "todas") {
            let (target, skip) = if next == Some("os") || next == Some("as") { (words.get(i + 2).map(String::as_str), 2) } else { (next, 1) };
            match target {
                Some("dia" | "dias") if !words.get(i + 2).is_some_and(|w| w.parse::<u32>().is_ok()) => repeat = Repeat::Daily,
                Some("semana" | "semanas") => repeat = Repeat::Weekly,
                Some("mês" | "mes" | "meses") => repeat = Repeat::Monthly,
                Some(w) if weekday(w).is_some() => {
                    repeat = Repeat::Weekly;
                    weekly_on = weekday(w);
                }
                Some("dia") => repeat = Repeat::Monthly,
                _ => continue,
            }
            used.extend(i..=i + skip);
        }
        match word.as_str() {
            "diariamente" => { repeat = Repeat::Daily; used.push(i); }
            "semanalmente" => { repeat = Repeat::Weekly; used.push(i); }
            "mensalmente" => { repeat = Repeat::Monthly; used.push(i); }
            "hoje" => { day = Some(now.date()); used.push(i); }
            "amanhã" | "amanha" => {
                let depois = i >= 2 && words[i - 1] == "de" && words[i - 2] == "depois";
                day = Some(now.date() + Duration::days(if depois { 2 } else { 1 }));
                used.push(i);
                if depois {
                    used.extend([i - 2, i - 1]);
                }
            }
            "daqui" | "em" if next == Some("a") || word == "em" => {
                // "daqui a 20 minutos", "em 2 horas", "daqui a meia hora".
                let start = if word == "daqui" { i + 2 } else { i + 1 };
                let (Some(amount), Some(unit)) = (words.get(start).and_then(|w| number(w)), words.get(start + 1)) else { continue };
                let delta = match unit.as_str() {
                    "minuto" | "minutos" | "min" => Duration::minutes(amount as i64),
                    "hora" | "horas" | "h" => Duration::hours(amount as i64),
                    "dia" | "dias" => Duration::days(amount as i64),
                    _ => continue,
                };
                let delta = if words.get(start) == Some(&"meia".to_string()) && unit.starts_with("hora") { Duration::minutes(30) } else { delta };
                when = Some(now + delta);
                used.extend(i..=start + 1);
            }
            "dia" if next.is_some_and(|n| n.parse::<u32>().is_ok()) => {
                let d: u32 = next?.parse().ok()?;
                let mut date = NaiveDate::from_ymd_opt(now.year(), now.month(), d);
                if date.is_none_or(|date| date < now.date()) {
                    let (y, m) = if now.month() == 12 { (now.year() + 1, 1) } else { (now.year(), now.month() + 1) };
                    date = NaiveDate::from_ymd_opt(y, m, d);
                }
                day = Some(date?);
                used.extend([i, i + 1]);
            }
            w if weekday(w).is_some() && repeat != Repeat::Weekly => {
                let target = weekday(w)?;
                let mut date = now.date() + Duration::days(1);
                while date.weekday() != target {
                    date += Duration::days(1);
                }
                day = Some(date);
                used.push(i);
                if i > 0 && matches!(words[i - 1].as_str(), "na" | "no" | "nesta" | "neste" | "próxima" | "proxima") {
                    used.push(i - 1);
                }
            }
            _ => {}
        }
    }
    let time = time_of(&words);
    if let Some((_, positions)) = &time {
        used.extend(positions);
    }
    let when = match (when, day, time.map(|t| t.0)) {
        (Some(when), _, _) => when,
        (None, Some(day), time) => day.and_time(time.unwrap_or(NaiveTime::from_hms_opt(9, 0, 0)?)),
        (None, None, Some(time)) => {
            let mut start = now.date();
            if let Some(target) = weekly_on {
                while start.weekday() != target {
                    start += Duration::days(1);
                }
            }
            let mut when = start.and_time(time);
            while when <= now {
                when = next(when, if repeat == Repeat::None { Repeat::Daily } else { repeat });
            }
            when
        }
        (None, None, None) => return None,
    };
    let when = if when <= now && repeat != Repeat::None {
        let mut when = when;
        while when <= now {
            when = next(when, repeat);
        }
        when
    } else {
        when
    };
    if when <= now {
        return None;
    }
    Some(Parsed { when, repeat, text: text_of(message, &words, &used) })
}

/// O que lembrar: o pedido sem o gatilho, a data, a hora e os conectivos do começo.
fn text_of(message: &str, words: &[String], used: &[usize]) -> String {
    let lower = message.to_lowercase();
    let trigger_words = TRIGGERS.iter().filter(|t| lower.contains(*t)).map(|t| t.split(' ').count()).max().unwrap_or(0);
    let start = TRIGGERS
        .iter()
        .filter_map(|t| {
            let parts: Vec<&str> = t.split(' ').collect();
            words.windows(parts.len()).position(|w| w.iter().zip(&parts).all(|(a, b)| a == b)).map(|p| p + parts.len())
        })
        .min()
        .unwrap_or(trigger_words);
    let kept: Vec<&str> = words
        .iter()
        .enumerate()
        .filter(|(i, _)| *i >= start && !used.contains(i))
        .map(|(_, w)| w.as_str())
        .collect();
    let mut kept: &[&str] = &kept;
    while let Some(first) = kept.first() {
        if matches!(*first, "de" | "para" | "pra" | "que" | "a" | "eu" | "do" | "da" | "às" | "as" | "o") {
            kept = &kept[1..];
        } else {
            break;
        }
    }
    let text = kept.join(" ");
    if text.is_empty() { "Lembrete".to_string() } else { text }
}

/// Próxima ocorrência de um lembrete que se repete.
pub fn next(when: NaiveDateTime, repeat: Repeat) -> NaiveDateTime {
    match repeat {
        Repeat::None | Repeat::Daily => when + Duration::days(1),
        Repeat::Weekly => when + Duration::days(7),
        Repeat::Monthly => {
            let (y, m) = if when.month() == 12 { (when.year() + 1, 1) } else { (when.year(), when.month() + 1) };
            let last = (28..=31).rev().find(|d| NaiveDate::from_ymd_opt(y, m, *d).is_some()).unwrap_or(28);
            NaiveDate::from_ymd_opt(y, m, when.day().min(last)).unwrap_or(when.date()).and_time(when.time())
        }
    }
}

/// Horário local agora, sem segundos fracionados.
pub fn now_local() -> NaiveDateTime {
    Local::now().naive_local().with_nanosecond_zero()
}
trait NoNanos {
    fn with_nanosecond_zero(self) -> Self;
}
impl NoNanos for NaiveDateTime {
    fn with_nanosecond_zero(self) -> Self {
        use chrono::Timelike;
        self.with_nanosecond(0).unwrap_or(self)
    }
}

/// Horário local em segundos Unix (ambíguo no horário de verão: a primeira ocorrência).
pub fn to_unix(when: NaiveDateTime) -> i64 {
    Local.from_local_datetime(&when).earliest().map(|t| t.timestamp()).unwrap_or_else(|| when.and_utc().timestamp())
}
pub fn from_unix(seconds: i64) -> NaiveDateTime {
    Local.timestamp_opt(seconds, 0).single().map(|t| t.naive_local()).unwrap_or_default()
}

const WEEKDAYS: [&str; 7] = ["segunda", "terça", "quarta", "quinta", "sexta", "sábado", "domingo"];

/// "sexta, 21/11 às 15:00", com "hoje" e "amanhã" quando couber.
pub fn describe(when: NaiveDateTime, now: NaiveDateTime) -> String {
    let day = if when.date() == now.date() {
        "hoje".to_string()
    } else if when.date() == now.date() + Duration::days(1) {
        "amanhã".to_string()
    } else {
        format!("{}, {}", WEEKDAYS[when.weekday().num_days_from_monday() as usize], when.format("%d/%m"))
    };
    format!("{day} às {}", when.format("%H:%M"))
}

pub fn repeat_label(repeat: Repeat, when: NaiveDateTime) -> &'static str {
    match repeat {
        Repeat::None => "",
        Repeat::Daily => " (todo dia)",
        Repeat::Weekly => match when.weekday() {
            Weekday::Mon => " (toda segunda)",
            Weekday::Tue => " (toda terça)",
            Weekday::Wed => " (toda quarta)",
            Weekday::Thu => " (toda quinta)",
            Weekday::Fri => " (toda sexta)",
            Weekday::Sat => " (todo sábado)",
            Weekday::Sun => " (todo domingo)",
        },
        Repeat::Monthly => " (todo mês)",
    }
}

/// Resposta da Ayla ao criar o lembrete.
pub fn confirmation(parsed: &Parsed, now: NaiveDateTime) -> String {
    format!(
        "Combinado! Vou te lembrar de **{}** {}{}. Você vê, adia ou cancela em Configurações › Lembretes.",
        parsed.text,
        describe(parsed.when, now),
        repeat_label(parsed.repeat, parsed.when)
    )
}

/// Pedido ao modelo local quando as regras não entendem (JSON validado).
pub fn model_request(message: &str, now: NaiveDateTime) -> Vec<serde_json::Value> {
    let system = format!(
        "Você extrai lembretes. Agora é {} ({}). Devolva só JSON com: quando (AAAA-MM-DDTHH:MM, horário local, no futuro), repetir (none, daily, weekly ou monthly) e texto (o que lembrar, curto, sem a data). Sem hora citada, use 09:00.",
        now.format("%Y-%m-%dT%H:%M"),
        WEEKDAYS[now.weekday().num_days_from_monday() as usize]
    );
    vec![serde_json::json!({"role":"system","content":system}), serde_json::json!({"role":"user","content":message})]
}
pub fn model_schema() -> serde_json::Value {
    serde_json::json!({"type":"object","properties":{
        "quando":{"type":"string","minLength":16,"maxLength":16},
        "repetir":{"type":"string","enum":["none","daily","weekly","monthly"]},
        "texto":{"type":"string","minLength":1,"maxLength":200}
    },"required":["quando","repetir","texto"]})
}
pub fn parse_model(raw: &str, now: NaiveDateTime) -> Option<Parsed> {
    let value: serde_json::Value = serde_json::from_str(raw.trim()).ok()?;
    let when = NaiveDateTime::parse_from_str(value["quando"].as_str()?, "%Y-%m-%dT%H:%M").ok()?;
    let text = value["texto"].as_str()?.trim().to_string();
    (when > now && !text.is_empty()).then(|| Parsed { when, repeat: Repeat::parse(value["repetir"].as_str().unwrap_or("none")), text })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Sexta-feira, 20/11/2026, 10:00.
    fn now() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 20).unwrap().and_hms_opt(10, 0, 0).unwrap()
    }
    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, min, 0).unwrap()
    }
    fn parsed(message: &str) -> Parsed {
        parse(message, now()).unwrap_or_else(|| panic!("não entendeu: {message}"))
    }

    #[test]
    fn reconhece_pedidos_sem_confundir_com_memoria() {
        assert!(is_request("me lembra às 15h de ligar pro médico"));
        assert!(is_request("Me avisa amanhã às 9 da reunião"));
        assert!(!is_request("lembre que eu gosto de café"));
        assert!(!is_request("qual o prazo do contrato?"));
    }

    #[test]
    fn entende_horas_e_dias() {
        let p = parsed("me lembra às 15h de ligar pro médico");
        assert_eq!((p.when, p.repeat, p.text.as_str()), (at(2026, 11, 20, 15, 0), Repeat::None, "ligar pro médico"));
        assert_eq!(parsed("me lembra às 9h de tomar remédio").when, at(2026, 11, 21, 9, 0), "hora passada vai para amanhã");
        assert_eq!(parsed("me avisa amanhã às 9 da reunião").when, at(2026, 11, 21, 9, 0));
        assert_eq!(parsed("me lembra amanhã às 3 da tarde de buscar o carro").when, at(2026, 11, 21, 15, 0));
        assert_eq!(parsed("me lembra depois de amanhã de pagar o aluguel").when, at(2026, 11, 22, 9, 0));
        assert_eq!(parsed("me lembra na segunda às 8h30 de enviar o relatório").when, at(2026, 11, 23, 8, 30));
        assert_eq!(parsed("me lembra sexta de ligar pra mãe").when, at(2026, 11, 27, 9, 0), "a próxima sexta");
        assert_eq!(parsed("me lembra dia 25 de pagar a fatura").when, at(2026, 11, 25, 9, 0));
        assert_eq!(parsed("me lembra dia 5 às 14:00 da consulta").when, at(2026, 12, 5, 14, 0));
        assert_eq!(parsed("me lembra daqui a 20 minutos de tirar o bolo").when, at(2026, 11, 20, 10, 20));
        assert_eq!(parsed("me lembra em 2 horas de alongar").when, at(2026, 11, 20, 12, 0));
        assert_eq!(parsed("me lembra ao meio-dia de almoçar").when, at(2026, 11, 20, 12, 0));
        assert_eq!(parsed("me lembra amanhã às 9 da reunião").text, "reunião");
        assert!(parse("me lembra de comprar pão", now()).is_none(), "sem data: vai para o modelo");
    }

    #[test]
    fn entende_repeticoes() {
        let daily = parsed("me lembra todo dia às 22h de tomar remédio");
        assert_eq!((daily.when, daily.repeat, daily.text.as_str()), (at(2026, 11, 20, 22, 0), Repeat::Daily, "tomar remédio"));
        let weekly = parsed("me lembra toda segunda às 7h de ir pra academia");
        assert_eq!((weekly.when, weekly.repeat), (at(2026, 11, 23, 7, 0), Repeat::Weekly));
        let monthly = parsed("me lembra todo mês dia 10 de pagar o condomínio");
        assert_eq!((monthly.when, monthly.repeat), (at(2026, 12, 10, 9, 0), Repeat::Monthly));
        assert_eq!(next(at(2026, 1, 31, 9, 0), Repeat::Monthly), at(2026, 2, 28, 9, 0));
        assert_eq!(next(at(2026, 12, 15, 9, 0), Repeat::Monthly), at(2027, 1, 15, 9, 0));
    }

    #[test]
    fn confirma_e_aceita_o_modelo() {
        let p = parsed("me lembra às 15h de ligar pro médico");
        assert_eq!(confirmation(&p, now()), "Combinado! Vou te lembrar de **ligar pro médico** hoje às 15:00. Você vê, adia ou cancela em Configurações › Lembretes.");
        assert_eq!(describe(at(2026, 11, 24, 8, 0), now()), "terça, 24/11 às 08:00");
        let model = parse_model(r#"{"quando":"2026-11-21T18:00","repetir":"weekly","texto":"comprar pão"}"#, now()).unwrap();
        assert_eq!((model.when, model.repeat, model.text.as_str()), (at(2026, 11, 21, 18, 0), Repeat::Weekly, "comprar pão"));
        assert!(parse_model(r#"{"quando":"2026-11-19T18:00","repetir":"none","texto":"x"}"#, now()).is_none(), "passado");
        assert!(parse_model("não é json", now()).is_none());
    }
}
