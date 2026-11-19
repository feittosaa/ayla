//! Prepara texto para síntese de voz em português do Brasil: números, moeda,
//! datas, horas e porcentagens por extenso; sem markdown, emojis ou símbolos.
//! Determinístico, sem modelo: os motores de voz leem mal "R$ 87,90" e "03/11".

const UNITS: [&str; 20] = [
    "zero", "um", "dois", "três", "quatro", "cinco", "seis", "sete", "oito", "nove", "dez", "onze",
    "doze", "treze", "catorze", "quinze", "dezesseis", "dezessete", "dezoito", "dezenove",
];
const TENS: [&str; 10] = [
    "", "", "vinte", "trinta", "quarenta", "cinquenta", "sessenta", "setenta", "oitenta", "noventa",
];
const HUNDREDS: [&str; 10] = [
    "", "cento", "duzentos", "trezentos", "quatrocentos", "quinhentos", "seiscentos", "setecentos",
    "oitocentos", "novecentos",
];
const MONTHS: [&str; 12] = [
    "janeiro", "fevereiro", "março", "abril", "maio", "junho", "julho", "agosto", "setembro",
    "outubro", "novembro", "dezembro",
];

fn feminine(word: &str) -> String {
    match word {
        "um" => "uma".into(),
        "dois" => "duas".into(),
        w if w.ends_with("entos") => w.replace("entos", "entas"),
        w => w.into(),
    }
}
/// Até 999, no gênero indicado.
fn below_thousand(n: u64, fem: bool) -> String {
    let unit = |w: &str| if fem { feminine(w) } else { w.to_string() };
    if n == 100 {
        return "cem".into();
    }
    let mut parts = Vec::new();
    if n >= 100 {
        parts.push(unit(HUNDREDS[(n / 100) as usize]));
    }
    let rest = n % 100;
    if rest >= 20 {
        parts.push(TENS[(rest / 10) as usize].to_string());
        if rest % 10 > 0 {
            parts.push(unit(UNITS[(rest % 10) as usize]));
        }
    } else if rest > 0 || n == 0 {
        parts.push(unit(UNITS[rest as usize]));
    }
    parts.join(" e ")
}
/// Número inteiro por extenso (até a casa dos trilhões).
pub fn number_words(n: u64, fem: bool) -> String {
    if n < 1000 {
        return below_thousand(n, fem);
    }
    let scales: [(u64, &str, &str); 4] = [
        (1_000_000_000_000, "um trilhão", "trilhões"),
        (1_000_000_000, "um bilhão", "bilhões"),
        (1_000_000, "um milhão", "milhões"),
        (1_000, "mil", "mil"),
    ];
    let mut parts: Vec<String> = Vec::new();
    let mut rest = n;
    for (value, one, many) in scales {
        let count = rest / value;
        if count == 0 {
            continue;
        }
        rest %= value;
        parts.push(match (count, value) {
            (1, _) => one.to_string(),
            (_, 1_000) => format!("{} mil", below_thousand(count, fem)),
            _ => format!("{} {many}", number_words(count, false)),
        });
    }
    if rest > 0 {
        // "mil e duzentos", "mil e trinta", mas "mil duzentos e trinta".
        let joiner = if rest < 100 || rest % 100 == 0 { " e " } else { " " };
        return format!("{}{joiner}{}", parts.join(" "), below_thousand(rest, fem));
    }
    parts.join(" ")
}

fn digits(s: &str) -> Option<u64> {
    (!s.is_empty() && s.len() <= 15 && s.chars().all(|c| c.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}
/// "1.234" ou "1234" → 1234; recusa separadores mal posicionados.
fn integer(s: &str) -> Option<u64> {
    if s.contains('.') {
        let groups: Vec<&str> = s.split('.').collect();
        let ok = groups[0].len() <= 3 && groups[1..].iter().all(|g| g.len() == 3);
        return ok.then(|| digits(&groups.concat())).flatten();
    }
    digits(s)
}
fn money(value: &str) -> Option<String> {
    let (whole, cents) = match value.split_once(',') {
        Some((w, c)) if c.len() == 2 => (integer(w)?, digits(c)?),
        None => (integer(value)?, 0),
        _ => return None,
    };
    let reais = match whole {
        0 => String::new(),
        1 => "um real".into(),
        n if n % 1_000_000 == 0 => format!("{} de reais", number_words(n, false)),
        n => format!("{} reais", number_words(n, false)),
    };
    let centavos = match cents {
        0 => String::new(),
        1 => "um centavo".into(),
        c => format!("{} centavos", number_words(c, false)),
    };
    Some(match (reais.is_empty(), centavos.is_empty()) {
        (false, false) => format!("{reais} e {centavos}"),
        (true, false) => centavos,
        _ if whole == 0 => "zero real".into(),
        _ => reais,
    })
}
fn date(token: &str) -> Option<String> {
    let parts: Vec<&str> = token.split('/').collect();
    if !(2..=3).contains(&parts.len()) || parts.iter().any(|p| p.len() > 4) {
        return None;
    }
    let day = digits(parts[0])?;
    let month = digits(parts[1])?;
    if !(1..=31).contains(&day) || !(1..=12).contains(&month) {
        return None;
    }
    let day = if day == 1 { "primeiro".into() } else { number_words(day, false) };
    let mut out = format!("{day} de {}", MONTHS[(month - 1) as usize]);
    if let Some(year) = parts.get(2) {
        let mut y = digits(year)?;
        if year.len() == 2 {
            y += 2000;
        }
        out.push_str(&format!(" de {}", number_words(y, false)));
    }
    Some(out)
}
fn time(token: &str) -> Option<String> {
    let (h, m) = if let Some((h, m)) = token.split_once(':') {
        (digits(h)?, digits(m)?)
    } else if let Some((h, m)) = token.split_once('h') {
        (digits(h)?, if m.is_empty() { 0 } else { digits(m)? })
    } else {
        return None;
    };
    if h > 23 || m > 59 || token.split_once(':').is_some_and(|(_, m)| m.len() != 2) {
        return None;
    }
    let hours = number_words(h, true);
    Some(match (h, m) {
        (1, 0) => "uma hora".into(),
        (_, 0) => format!("{hours} horas"),
        _ => format!("{hours} e {}", number_words(m, false)),
    })
}
fn decimal(token: &str) -> Option<String> {
    let (whole, frac) = token.split_once(',')?;
    let whole = integer(whole)?;
    if frac.is_empty() || frac.len() > 3 || !frac.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let frac = if frac.starts_with('0') {
        frac.chars().map(|c| UNITS[c.to_digit(10).unwrap() as usize]).collect::<Vec<_>>().join(" ")
    } else {
        number_words(digits(frac)?, false)
    };
    Some(format!("{} vírgula {frac}", number_words(whole, false)))
}
fn number_token(token: &str) -> Option<String> {
    if let Some(p) = token.strip_suffix('%') {
        let value = decimal(p).or_else(|| integer(p).map(|n| number_words(n, false)))?;
        return Some(format!("{value} por cento"));
    }
    if token.contains('/') {
        return date(token);
    }
    if token.contains(':') || (token.contains('h') && token.starts_with(|c: char| c.is_ascii_digit())) {
        return time(token);
    }
    if token.contains(',') {
        return decimal(token);
    }
    integer(token).map(|n| number_words(n, false))
}

/// Remove marcação de markdown mantendo o texto legível.
fn strip_markdown(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let mut line = line.trim_start();
        line = line.trim_start_matches('#').trim_start_matches('>').trim_start();
        for bullet in ["- ", "* ", "• "] {
            if let Some(rest) = line.strip_prefix(bullet) {
                line = rest;
            }
        }
        let mut cleaned = String::new();
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '*' | '_' | '`' | '~' => {}
                // [texto](link) → texto
                ']' if chars.peek() == Some(&'(') => {
                    for c in chars.by_ref() {
                        if c == ')' {
                            break;
                        }
                    }
                }
                '[' => {}
                _ => cleaned.push(c),
            }
        }
        let cleaned = cleaned.trim();
        if !cleaned.is_empty() {
            out.push_str(cleaned);
            // Linhas viram frases para a voz pausar entre itens.
            if !cleaned.ends_with(['.', '!', '?', ':', ';', ',']) {
                out.push('.');
            }
            out.push(' ');
        }
    }
    out
}

/// Texto pronto para o motor de voz.
pub fn prepare(text: &str) -> String {
    let text = strip_markdown(text);
    let mut out = String::new();
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut i = 0;
    while i < tokens.len() {
        let raw = tokens[i];
        // Separa pontuação final do token ("87,90." → "87,90" + ".").
        let core = raw.trim_end_matches(['.', ',', ';', ':', '!', '?', ')']);
        let tail = &raw[core.len()..];
        let (lead, core) = core.split_at(core.len() - core.trim_start_matches(['(', '"', '\'']).len());
        let mut spoken: Option<String> = None;
        if core == "R$" && i + 1 < tokens.len() {
            let next = tokens[i + 1];
            let value = next.trim_end_matches(['.', ';', ':', '!', '?', ')']);
            let value = value.strip_suffix(',').unwrap_or(value);
            if let Some(words) = money(value) {
                out.push_str(&format!("{lead}{words}{} ", &next[value.len()..]));
                i += 2;
                continue;
            }
        } else if let Some(value) = core.strip_prefix("R$") {
            spoken = money(value);
        } else if core.starts_with(|c: char| c.is_ascii_digit()) {
            spoken = number_token(core);
        }
        out.push_str(lead);
        out.push_str(spoken.as_deref().unwrap_or(core));
        out.push_str(tail);
        out.push(' ');
        i += 1;
    }
    // Sem emojis e símbolos; nunca começar com "-" (seria lido como opção pelo motor).
    let filtered: String = out
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() || ".,;:!?'\"()-%".contains(c) {
                c
            } else {
                ' '
            }
        })
        .collect();
    let mut spoken = filtered.split_whitespace().collect::<Vec<_>>().join(" ");
    for mark in [".", ",", ";", ":", "!", "?", ")"] {
        spoken = spoken.replace(&format!(" {mark}"), mark);
    }
    while spoken.contains("..") || spoken.contains(".,") {
        spoken = spoken.replace("..", ".").replace(".,", ".");
    }
    let spoken = spoken.trim_start_matches(['-', ' ', '.', ',']).to_string();
    if spoken.chars().any(char::is_alphanumeric) { spoken } else { String::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numbers_follow_portuguese_rules() {
        for (n, words) in [
            (0, "zero"), (16, "dezesseis"), (21, "vinte e um"), (100, "cem"), (101, "cento e um"),
            (200, "duzentos"), (999, "novecentos e noventa e nove"), (1000, "mil"), (1001, "mil e um"),
            (1200, "mil e duzentos"), (1230, "mil duzentos e trinta"), (2026, "dois mil e vinte e seis"),
            (21000, "vinte e um mil"), (1_000_000, "um milhão"), (2_500_000, "dois milhões quinhentos mil"),
        ] {
            assert_eq!(number_words(n, false), words, "{n}");
        }
        assert_eq!(number_words(202, true), "duzentas e duas");
    }
    #[test]
    fn money_dates_times_and_percentages_are_spoken() {
        assert_eq!(
            prepare("A compra de R$ 87,90 foi aprovada em 03/11 na Padaria Bom Pão."),
            "A compra de oitenta e sete reais e noventa centavos foi aprovada em três de novembro na Padaria Bom Pão."
        );
        assert_eq!(prepare("Custa R$1.200,00, ok?"), "Custa mil e duzentos reais, ok?");
        assert_eq!(prepare("R$ 0,50 e R$ 1"), "cinquenta centavos e um real.");
        assert_eq!(prepare("Consulta às 9h30 e às 14:05; volte às 2h."), "Consulta às nove e trinta e às catorze e cinco; volte às duas horas.");
        assert_eq!(prepare("Em 01/01/2027 subiu 35% (ou 3,5%)."), "Em primeiro de janeiro de dois mil e vinte e sete subiu trinta e cinco por cento (ou três vírgula cinco por cento).");
        assert_eq!(prepare("Versão 3,05 e 12/45 e 25:00"), "Versão três vírgula zero cinco e 12 45 e 25:00.");
    }
    #[test]
    fn markdown_emojis_and_option_like_text_are_removed() {
        assert_eq!(
            prepare("## Dicas 😊\n- **Arroz** com feijão\n- Veja [o site](https://x.y) ✨"),
            "Dicas. Arroz com feijão. Veja o site."
        );
        assert_eq!(prepare("--help agora"), "help agora.");
        assert_eq!(prepare("🌸💖"), "");
        assert_eq!(prepare("Feito. 😊 Até logo... ✨"), "Feito. Até logo.");
    }
}
