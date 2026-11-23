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
    /// Todo ano (aniversários).
    Yearly,
    /// Segunda a sexta.
    Weekdays,
}
impl Repeat {
    pub fn as_str(self) -> &'static str {
        match self {
            Repeat::None => "none",
            Repeat::Daily => "daily",
            Repeat::Weekly => "weekly",
            Repeat::Monthly => "monthly",
            Repeat::Yearly => "yearly",
            Repeat::Weekdays => "weekdays",
        }
    }
    pub fn parse(value: &str) -> Self {
        match value {
            "daily" => Repeat::Daily,
            "weekly" => Repeat::Weekly,
            "monthly" => Repeat::Monthly,
            "yearly" => Repeat::Yearly,
            "weekdays" => Repeat::Weekdays,
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
    "me lembra", "me lembre", "lembre-me", "lembra-me", "me lembrar", "me avisa", "me avise", "me notifica", "me notifique",
    "manda um lembrete", "mande um lembrete", "manda um aviso", "mande um aviso", "cria um lembrete",
    "crie um lembrete", "criar lembrete", "novo lembrete", "lembrete para", "lembrete pra", "lembrete de",
];

/// Pedido de lembrete ("me lembra…", "me avisa…", "cria um lembrete…").
pub fn is_request(message: &str) -> bool {
    let text = message.to_lowercase();
    TRIGGERS.iter().any(|t| text.contains(t))
        // "configura o aviso daqui 10 segundos", "notificação amanhã às 9": palavra de aviso + quando.
        || (NOUNS.iter().any(|n| text.contains(n)) && parse(message, now_local()).is_some())
}

/// Palavras de aviso que, junto com uma data ou hora entendida, também pedem um lembrete.
const NOUNS: &[&str] = &["lembrete", "aviso", "notificação", "notificacao", "alarme"];
/// Palavras que não fazem parte do que lembrar.
const FILLER: &[&str] = &[
    "lembrete", "aviso", "notificação", "notificacao", "alarme", "automático", "automatico", "configura", "configure",
    "configurar", "cria", "crie", "criar", "manda", "mande", "me", "um", "uma", "por", "favor", "ayla",
];

/// Texto genérico: a Ayla tenta descobrir o assunto pela conversa.
pub fn generic(text: &str) -> bool {
    text == "Lembrete"
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
                Some("ano" | "anos") => repeat = Repeat::Yearly,
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
            "anualmente" => { repeat = Repeat::Yearly; used.push(i); }
            // "nos dias úteis", "em dias úteis", "todos os dias úteis".
            "úteis" | "uteis" if i > 0 && words[i - 1] == "dias" => {
                repeat = Repeat::Weekdays;
                used.extend([i - 1, i]);
                if i > 1 && matches!(words[i - 2].as_str(), "nos" | "em" | "os") { used.push(i - 2); }
                if i > 2 && words[i - 3] == "todos" { used.push(i - 3); }
            }
            "hoje" => { day = Some(now.date()); used.push(i); }
            "amanhã" | "amanha" => {
                let depois = i >= 2 && words[i - 1] == "de" && words[i - 2] == "depois";
                day = Some(now.date() + Duration::days(if depois { 2 } else { 1 }));
                used.push(i);
                if depois {
                    used.extend([i - 2, i - 1]);
                }
            }
            "daqui" | "em" if next == Some("a") || word == "em" || next.and_then(number).is_some() => {
                // "daqui a 20 minutos", "daqui 10 seg", "em 2 horas", "daqui a meia hora".
                let start = if word == "daqui" && next == Some("a") { i + 2 } else { i + 1 };
                let (Some(amount), Some(unit)) = (words.get(start).and_then(|w| number(w)), words.get(start + 1)) else { continue };
                let delta = match unit.as_str() {
                    "segundo" | "segundos" | "seg" | "segs" | "s" => Duration::seconds(amount as i64),
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
    let mut when = when;
    while repeat == Repeat::Weekdays && matches!(when.weekday(), Weekday::Sat | Weekday::Sun) {
        when += Duration::days(1);
    }
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
        .filter(|w| !FILLER.contains(w))
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
    if text.is_empty() || matches!(text.as_str(), "o" | "a" | "de") { "Lembrete".to_string() } else { text }
}

/// Próxima ocorrência de um lembrete que se repete.
pub fn next(when: NaiveDateTime, repeat: Repeat) -> NaiveDateTime {
    match repeat {
        Repeat::None | Repeat::Daily => when + Duration::days(1),
        Repeat::Weekly => when + Duration::days(7),
        Repeat::Monthly => {
            let (y, m) = if when.month() == 12 { (when.year() + 1, 1) } else { (when.year(), when.month() + 1) };
            on_day(y, m, when.day()).and_time(when.time())
        }
        Repeat::Yearly => on_day(when.year() + 1, when.month(), when.day()).and_time(when.time()),
        Repeat::Weekdays => {
            let mut next = when + Duration::days(1);
            while matches!(next.weekday(), Weekday::Sat | Weekday::Sun) {
                next += Duration::days(1);
            }
            next
        }
    }
}

/// Dia `day` do mês, ou o último dia quando o mês é mais curto (31 → 30, 29/02 → 28/02).
fn on_day(year: i32, month: u32, day: u32) -> NaiveDate {
    (1..=day.min(31)).rev().find_map(|d| NaiveDate::from_ymd_opt(year, month, d)).unwrap_or_default()
}

/// Ocorrências de um item que começa em `start` e se repete, entre `from` e `to` (inclusive).
/// Contadas a partir do início: um mensal do dia 31 cai no dia 30 em abril e volta ao 31 em maio.
pub fn occurrences(start: NaiveDateTime, repeat: Repeat, from: NaiveDate, to: NaiveDate) -> Vec<NaiveDateTime> {
    let first = from.max(start.date());
    if first > to {
        return Vec::new();
    }
    let (time, anchor) = (start.time(), start.date());
    let mut out = Vec::new();
    match repeat {
        Repeat::None => out.push(start),
        Repeat::Daily | Repeat::Weekly | Repeat::Weekdays => {
            let mut day = first;
            while day <= to {
                let keep = match repeat {
                    Repeat::Weekly => (day - anchor).num_days() % 7 == 0,
                    Repeat::Weekdays => !matches!(day.weekday(), Weekday::Sat | Weekday::Sun),
                    _ => true,
                };
                if keep {
                    out.push(day.and_time(time));
                }
                day += Duration::days(1);
            }
        }
        Repeat::Monthly => {
            let (mut y, mut m) = (first.year(), first.month());
            while (y, m) <= (to.year(), to.month()) {
                out.push(on_day(y, m, anchor.day()).and_time(time));
                (y, m) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
            }
        }
        Repeat::Yearly => {
            for y in first.year()..=to.year() {
                out.push(on_day(y, anchor.month(), anchor.day()).and_time(time));
            }
        }
    }
    out.retain(|when| when.date() >= first && when.date() <= to);
    out
}

/// Maior antecedência de um aviso: 365 dias, em minutos.
pub const MAX_ALERT_MINUTES: i64 = 365 * 24 * 60;

/// Próximo aviso depois de `after`, com a ocorrência a que ele se refere. `alerts` são as
/// antecedências em minutos (0 = na hora; 60 = uma hora antes; 1440 = um dia antes).
pub fn next_alert(start: NaiveDateTime, repeat: Repeat, alerts: &[i64], after: NaiveDateTime) -> Option<(NaiveDateTime, NaiveDateTime)> {
    let longest = alerts.iter().copied().max()?;
    // O aviso vem antes da ocorrência: basta olhar as ocorrências depois de `after`.
    let from = after.date();
    let to = from + Duration::days(370) + Duration::minutes(longest);
    occurrences(start, repeat, from, to).into_iter()
        .flat_map(|occurrence| alerts.iter().map(move |minutes| (occurrence - Duration::minutes(*minutes), occurrence)))
        .filter(|(alert, _)| *alert > after)
        .min()
}

/// "na hora", "30 minutos antes", "1 hora antes", "2 dias antes".
pub fn alert_label(minutes: i64) -> String {
    let plural = |n: i64, one: &str, many: &str| format!("{n} {} antes", if n == 1 { one } else { many });
    match minutes {
        0 => "na hora".into(),
        m if m % 1440 == 0 => plural(m / 1440, "dia", "dias"),
        m if m % 60 == 0 => plural(m / 60, "hora", "horas"),
        m => plural(m, "minuto", "minutos"),
    }
}

/// Agenda para a conversa local: as ocorrências de ontem até `days` dias à frente, com os avisos.
/// `items` são (texto, início, repetição, dia todo, avisos).
pub fn agenda_block(items: &[(String, NaiveDateTime, Repeat, bool, Vec<i64>)], now: NaiveDateTime, days: i64) -> String {
    let (from, to) = (now.date() - Duration::days(1), now.date() + Duration::days(days));
    let mut lines: Vec<(NaiveDateTime, String)> = Vec::new();
    for (text, start, repeat, all_day, alerts) in items {
        for when in occurrences(*start, *repeat, from, to) {
            let day = describe(when, now);
            let day = if *all_day { format!("{} (dia todo)", day.split(" às ").next().unwrap_or(&day)) } else { day };
            let mut line = format!("- {day}: {}", text.replace('\n', " "));
            if *repeat != Repeat::None { line.push_str(repeat_label(*repeat, *start)); }
            if alerts.is_empty() {
                line.push_str(" · sem aviso");
            } else {
                let mut sorted = alerts.clone();
                sorted.sort_unstable_by(|a, b| b.cmp(a));
                line.push_str(&format!(" · aviso {}", sorted.iter().map(|m| alert_label(*m)).collect::<Vec<_>>().join(" e ")));
            }
            if when < now { line.push_str(" · já passou"); }
            lines.push((when, line));
        }
    }
    lines.sort();
    let shown: Vec<String> = lines.iter().take(80).map(|(_, l)| l.clone()).collect();
    let header = format!(
        "\n\nAgenda da pessoa (calendário da Athena, horário local; agora é {}, {}), de ontem até {} dias à frente. Use estes itens quando ela perguntar sobre agenda, compromissos, eventos, lembretes ou avisos; não invente itens. Para criar, ela pode pedir \"me lembra…\" ou usar o calendário da Athena.",
        WEEKDAYS[now.weekday().num_days_from_monday() as usize], now.format("%d/%m/%Y %H:%M"), days
    );
    if shown.is_empty() {
        format!("{header}\nNada marcado nesse período.")
    } else {
        let more = lines.len().saturating_sub(shown.len());
        format!("{header}\n{}{}", shown.join("\n"), if more > 0 { format!("\n(e mais {more} itens)") } else { String::new() })
    }
}

/// Primeira ocorrência depois de `after`, ou `None` se não há mais (os avisos usam `next_alert`).
#[cfg(test)]
fn next_after(start: NaiveDateTime, repeat: Repeat, after: NaiveDateTime) -> Option<NaiveDateTime> {
    if repeat == Repeat::None {
        return (start > after).then_some(start);
    }
    let from = after.date().max(start.date());
    occurrences(start, repeat, from, from + Duration::days(370)).into_iter().find(|when| *when > after)
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
        Repeat::Yearly => " (todo ano)",
        Repeat::Weekdays => " (dias úteis)",
    }
}

/// Resposta da Ayla ao criar o lembrete.
pub fn confirmation(parsed: &Parsed, now: NaiveDateTime) -> String {
    format!(
        "Combinado! Vou te lembrar de **{}** {}{}. Ele aparece no calendário da Athena; você adia ou cancela em Configurações › Lembretes.",
        parsed.text,
        describe(parsed.when, now),
        repeat_label(parsed.repeat, parsed.when)
    )
}

/// Pedido ao modelo local quando as regras não entendem (JSON validado).
pub fn model_request(message: &str, now: NaiveDateTime) -> Vec<serde_json::Value> {
    let system = format!(
        "Você extrai lembretes. Agora é {} ({}). Devolva só JSON com: quando (AAAA-MM-DDTHH:MM, horário local, no futuro), repetir (none, daily, weekly, monthly, yearly ou weekdays, este para dias úteis) e texto (o que lembrar, curto, sem a data). Sem hora citada, use 09:00.",
        now.format("%Y-%m-%dT%H:%M"),
        WEEKDAYS[now.weekday().num_days_from_monday() as usize]
    );
    vec![serde_json::json!({"role":"system","content":system}), serde_json::json!({"role":"user","content":message})]
}
/// Assunto do lembrete pela conversa recente ("uma notificação pra beber água" antes de
/// "configura o aviso daqui 10 segundos").
pub fn subject_request(message: &str, recent: &str) -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({"role":"system","content":"Diga em poucas palavras do que a pessoa quer ser lembrada, pela conversa. Devolva só JSON com texto (ex.: \"beber água\"). Se não houver assunto, use \"Lembrete\"."}),
        serde_json::json!({"role":"user","content":format!("Conversa recente:\n{recent}\n\nPedido: {message}")}),
    ]
}
pub fn subject_schema() -> serde_json::Value {
    serde_json::json!({"type":"object","properties":{"texto":{"type":"string","minLength":1,"maxLength":120}},"required":["texto"]})
}
pub fn parse_subject(raw: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(raw.trim()).ok()?;
    Some(value["texto"].as_str()?.trim().to_string()).filter(|t| !t.is_empty())
}

pub fn model_schema() -> serde_json::Value {
    serde_json::json!({"type":"object","properties":{
        "quando":{"type":"string","minLength":16,"maxLength":16},
        "repetir":{"type":"string","enum":["none","daily","weekly","monthly","yearly","weekdays"]},
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
    fn ocorrencias_contam_a_partir_do_inicio() {
        let d = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        let start = at(2026, 1, 31, 15, 0);
        let monthly: Vec<_> = occurrences(start, Repeat::Monthly, d(2026, 1, 1), d(2026, 5, 31)).iter().map(|w| w.date()).collect();
        assert_eq!(monthly, [d(2026, 1, 31), d(2026, 2, 28), d(2026, 3, 31), d(2026, 4, 30), d(2026, 5, 31)], "sem escorregar para o 28");
        let birthday = at(2024, 2, 29, 9, 0);
        let yearly: Vec<_> = occurrences(birthday, Repeat::Yearly, d(2025, 1, 1), d(2028, 12, 31)).iter().map(|w| w.date()).collect();
        assert_eq!(yearly, [d(2025, 2, 28), d(2026, 2, 28), d(2027, 2, 28), d(2028, 2, 29)]);
        // Sexta 20/11 → dias úteis até quarta 25/11, sem sábado e domingo.
        let weekdays = occurrences(now(), Repeat::Weekdays, d(2026, 11, 1), d(2026, 11, 25));
        assert_eq!(weekdays.iter().map(|w| w.day()).collect::<Vec<_>>(), [20, 23, 24, 25]);
        let weekly = occurrences(now(), Repeat::Weekly, d(2026, 11, 21), d(2026, 12, 10));
        assert_eq!(weekly.iter().map(|w| w.day()).collect::<Vec<_>>(), [27, 4]);
        assert!(occurrences(now(), Repeat::None, d(2026, 11, 21), d(2026, 11, 30)).is_empty(), "fora do período");
        assert_eq!(occurrences(now(), Repeat::None, d(2026, 11, 20), d(2026, 11, 20)), [now()]);
        assert_eq!(next_after(now(), Repeat::Weekdays, at(2026, 11, 20, 10, 0)), Some(at(2026, 11, 23, 10, 0)), "pula o fim de semana");
        assert_eq!(next_after(start, Repeat::Monthly, at(2026, 2, 28, 15, 0)), Some(at(2026, 3, 31, 15, 0)));
        assert_eq!(next_after(now(), Repeat::None, now()), None, "único já passou");
    }

    #[test]
    fn avisos_antes_e_na_hora() {
        let start = at(2026, 11, 25, 15, 0);
        // Uma hora antes e na hora.
        let alerts = [60, 0];
        assert_eq!(next_alert(start, Repeat::None, &alerts, now()), Some((at(2026, 11, 25, 14, 0), start)));
        assert_eq!(next_alert(start, Repeat::None, &alerts, at(2026, 11, 25, 14, 0)), Some((start, start)));
        assert_eq!(next_alert(start, Repeat::None, &alerts, start), None, "acabou");
        // Semanal com aviso de 2 dias antes: o da semana seguinte vem antes do "na hora" desta.
        let weekly = next_alert(start, Repeat::Weekly, &[2880, 0], at(2026, 11, 25, 14, 59)).unwrap();
        assert_eq!(weekly, (start, start));
        assert_eq!(next_alert(start, Repeat::Weekly, &[2880, 0], start).unwrap(), (at(2026, 11, 30, 15, 0), at(2026, 12, 2, 15, 0)));
        assert_eq!(next_alert(start, Repeat::None, &[], now()), None, "sem aviso");
        assert_eq!([alert_label(0), alert_label(60), alert_label(180), alert_label(1440), alert_label(4320), alert_label(30)],
            ["na hora", "1 hora antes", "3 horas antes", "1 dia antes", "3 dias antes", "30 minutos antes"]);
    }

    #[test]
    fn agenda_para_a_conversa() {
        let items = vec![
            ("Dentista".to_string(), at(2026, 11, 21, 15, 0), Repeat::None, false, vec![60, 0]),
            ("Academia".to_string(), at(2026, 11, 2, 7, 0), Repeat::Weekly, false, vec![]),
            ("Aniversário da Ana".to_string(), at(2026, 11, 22, 9, 0), Repeat::Yearly, true, vec![1440]),
            ("Velho".to_string(), at(2026, 10, 1, 9, 0), Repeat::None, false, vec![]),
        ];
        let block = agenda_block(&items, now(), 7);
        assert!(block.contains("- amanhã às 15:00: Dentista · aviso 1 hora antes e na hora"), "{block}");
        assert!(block.contains("- segunda, 23/11 às 07:00: Academia (toda segunda) · sem aviso"), "{block}");
        assert!(block.contains("- domingo, 22/11 (dia todo): Aniversário da Ana (todo ano) · aviso 1 dia antes"), "{block}");
        assert!(!block.contains("Velho"), "fora do período");
        assert!(block.find("Dentista").unwrap() < block.find("Aniversário").unwrap(), "em ordem de data");
        assert!(agenda_block(&[], now(), 7).contains("Nada marcado"));
    }

    #[test]
    fn entende_ano_e_dias_uteis() {
        assert_eq!(parsed("me lembra todo ano dia 5 do aniversário da Ana").repeat, Repeat::Yearly);
        let q = parsed("me lembra nos dias úteis às 8h de beber água");
        assert_eq!((q.repeat, q.when, q.text.as_str()), (Repeat::Weekdays, at(2026, 11, 23, 8, 0), "beber água"));
        assert_eq!(next(at(2026, 11, 20, 8, 0), Repeat::Weekdays), at(2026, 11, 23, 8, 0));
        let saturday = at(2026, 11, 21, 10, 0);
        assert_eq!(parse("me lembra nos dias úteis às 20h de revisar", saturday).unwrap().when, at(2026, 11, 23, 20, 0), "sábado pula para segunda");
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
    fn entende_segundos_e_avisos_sem_gatilho() {
        let p = parsed("ayla, me manda um lembrete teste daqui 10 seg");
        assert_eq!((p.when, p.text.as_str()), (at(2026, 11, 20, 10, 0) + Duration::seconds(10), "teste"));
        let q = parsed("configura o aviso automatico daqui 10 segundos");
        assert_eq!(q.when, at(2026, 11, 20, 10, 0) + Duration::seconds(10));
        assert!(generic(&q.text), "{}", q.text);
        assert!(is_request("ayla, me manda um lembrete teste daqui 10 seg"));
        assert!(is_request("configura o aviso automatico daqui 10 segundos"));
        assert!(!is_request("o aviso de privacidade é chato"), "sem data ou hora não é lembrete");
        assert_eq!(parse_subject(r#"{"texto":"beber água"}"#).as_deref(), Some("beber água"));
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
        assert_eq!(confirmation(&p, now()), "Combinado! Vou te lembrar de **ligar pro médico** hoje às 15:00. Ele aparece no calendário da Athena; você adia ou cancela em Configurações › Lembretes.");
        assert_eq!(describe(at(2026, 11, 24, 8, 0), now()), "terça, 24/11 às 08:00");
        let model = parse_model(r#"{"quando":"2026-11-21T18:00","repetir":"weekly","texto":"comprar pão"}"#, now()).unwrap();
        assert_eq!((model.when, model.repeat, model.text.as_str()), (at(2026, 11, 21, 18, 0), Repeat::Weekly, "comprar pão"));
        assert!(parse_model(r#"{"quando":"2026-11-19T18:00","repetir":"none","texto":"x"}"#, now()).is_none(), "passado");
        assert!(parse_model("não é json", now()).is_none());
    }
}
