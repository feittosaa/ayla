//! Memória longa da Ayla: fatos locais, busca híbrida e extração automática.
//! Lógica pura aqui; persistência em `store`, execução de modelos em `runtime`.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Similaridade mínima para uma memória entrar no contexto (EmbeddingGemma,
/// calibrado em 19/11/2026: perguntas sem relação chegaram a 0,41).
pub const RETRIEVAL_MIN: f64 = 0.42;
/// A partir daqui duas memórias são a mesma (paráfrases ficaram entre 0,90 e 0,98).
pub const DUPLICATE_MIN: f64 = 0.88;
/// Memórias parecidas o bastante para a extração decidir se substituem
/// (uma mudança de cidade ficou em 0,75 entre documentos).
pub const RELATED_MIN: f64 = 0.45;
pub const CONTEXT_LIMIT: usize = 5;
/// Tempo sem mensagens antes de extrair memórias de uma conversa.
pub const IDLE_SECONDS: u64 = 45;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Privacy {
    Sensitive,
    Uncertain,
    Public,
}
impl Privacy {
    pub fn as_str(self) -> &'static str {
        match self {
            Privacy::Sensitive => "sensitive",
            Privacy::Uncertain => "uncertain",
            Privacy::Public => "public",
        }
    }
    pub fn parse(value: &str) -> Privacy {
        match value {
            "sensitive" => Privacy::Sensitive,
            "public" => Privacy::Public,
            _ => Privacy::Uncertain,
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Fact {
    pub id: String,
    pub content: String,
    pub kind: String,
    pub privacy: Privacy,
    /// auto (extraída), explicit (pedido na conversa) ou manual (painel/nota).
    pub origin: String,
    pub conversation_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Projeto dono da memória; `None` é memória geral (pessoal).
    #[serde(default)]
    pub project_id: Option<String>,
}

pub fn encode(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|v| v.to_le_bytes()).collect()
}
pub fn decode(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.is_empty() || bytes.len() % 4 != 0 {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}
pub fn cosine(a: &[f32], b: &[f32]) -> Option<f64> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let (mut dot, mut na, mut nb) = (0f64, 0f64, 0f64);
    for (x, y) in a.iter().zip(b) {
        dot += f64::from(*x) * f64::from(*y);
        na += f64::from(*x) * f64::from(*x);
        nb += f64::from(*y) * f64::from(*y);
    }
    (na > 0.0 && nb > 0.0).then(|| dot / (na.sqrt() * nb.sqrt()))
}

/// Prefixos recomendados para EmbeddingGemma; outros modelos os toleram.
pub fn query_text(text: &str) -> String {
    format!("task: search result | query: {text}")
}
pub fn document_text(text: &str) -> String {
    format!("title: none | text: {text}")
}

const STOPWORDS: &[&str] = &[
    "usuário", "usuario", "para", "como", "qual", "quais", "quando", "onde", "você", "voce", "isso",
    "esse", "essa", "este", "esta", "mesmo", "minha", "meu", "meus", "minhas", "sobre", "pelo",
    "pela", "mais", "menos", "muito", "tenho", "tem", "estou", "está", "esta", "ayla", "que",
    "porque", "então", "entao", "algo", "fazer", "pode", "posso", "seria", "sabe", "lembra",
];
/// Palavras relevantes para busca textual.
pub(super) fn search_terms(text: &str) -> Vec<String> {
    let mut terms: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .map(|w| w.to_lowercase())
        .filter(|w| w.chars().count() >= 3 && !STOPWORDS.contains(&w.as_str()))
        .collect();
    terms.sort();
    terms.dedup();
    terms.truncate(12);
    terms
}

/// Consulta FTS5 segura: só palavras relevantes, cada uma entre aspas.
pub fn fts_query(text: &str) -> Option<String> {
    let terms = search_terms(text);
    (!terms.is_empty()).then(|| {
        terms
            .iter()
            .map(|t| format!("\"{t}\""))
            .collect::<Vec<_>>()
            .join(" OR ")
    })
}

/// Combina busca vetorial (já filtrada pelo limiar) e textual por fusão de ranks.
/// Resultados só textuais entram atrás dos vetoriais, nunca no lugar deles.
pub fn fuse(vector_hits: &[(String, f64)], text_hits: &[String], limit: usize) -> Vec<String> {
    let mut scores: Vec<(String, f64)> = Vec::new();
    let mut add = |id: &str, score: f64| match scores.iter_mut().find(|(i, _)| i == id) {
        Some(entry) => entry.1 += score,
        None => scores.push((id.to_string(), score)),
    };
    for (rank, (id, _)) in vector_hits.iter().enumerate() {
        add(id, 1.0 + 1.0 / (60.0 + rank as f64));
    }
    for (rank, id) in text_hits.iter().enumerate() {
        add(id, 1.0 / (60.0 + rank as f64));
    }
    scores.sort_by(|a, b| b.1.total_cmp(&a.1));
    scores.into_iter().take(limit).map(|(id, _)| id).collect()
}

/// Bloco acrescentado ao prompt de sistema em respostas locais.
pub fn context_block(facts: &[Fact]) -> Option<String> {
    if facts.is_empty() {
        return None;
    }
    let lines: Vec<String> = facts.iter().map(|f| format!("- {}", f.content)).collect();
    Some(format!(
        "\n\nMemórias locais sobre o usuário (use apenas quando forem relevantes ao pedido; não as recite nem invente outras):\n{}",
        lines.join("\n")
    ))
}

/// Memória mostrada na revisão de envio à nuvem e enviada junto, se a pessoa mantiver marcada.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Reference {
    pub id: String,
    pub content: String,
}

/// No máximo 5 referências por envio.
pub const MAX_REFERENCES: usize = 5;

/// Confere, no envio, que cada referência continua igual ao revisado, pública e visível nesta
/// conversa (memória geral ou do mesmo projeto). Sem isso, nada é enviado.
pub fn check_references(references: &[Reference], facts: &[Fact], project: Option<&str>) -> Result<(), String> {
    if references.len() > MAX_REFERENCES {
        return Err("Envie até 5 referências por pedido".into());
    }
    for reference in references {
        let ok = facts.iter().any(|f| {
            f.id == reference.id
                && f.content == reference.content
                && f.privacy == Privacy::Public
                && (f.project_id.is_none() || f.project_id.as_deref() == project)
        });
        if !ok {
            return Err("Uma referência mudou ou deixou de ser pública. Revise o envio de novo.".into());
        }
    }
    Ok(())
}

/// Bloco acrescentado às instruções enviadas à nuvem, só com as referências revisadas.
pub fn reference_block(references: &[Reference]) -> Option<String> {
    if references.is_empty() {
        return None;
    }
    let lines: Vec<String> = references.iter().map(|r| format!("- {}", r.content)).collect();
    Some(format!("\n\nInformações sobre o usuário que ele revisou para este pedido (use só se ajudarem):\n{}", lines.join("\n")))
}

/// Pedido explícito para lembrar ou esquecer: extrair logo após a resposta.
pub fn explicit_command(message: &str) -> bool {
    let lower = message.to_lowercase();
    [
        "lembre que", "lembra que", "lembre-se", "lembrar que", "guarde que", "anote que",
        "não esqueça", "nao esqueca", "esqueça", "esqueca", "pode esquecer",
    ]
    .iter()
    .any(|c| lower.contains(c))
}

pub struct Turn {
    pub role: String,
    pub content: String,
}

/// Mensagens para o modelo de conversa extrair memórias com saída JSON restrita.
pub fn extraction_request(turns: &[Turn], related: &[Fact]) -> Vec<Value> {
    let mut dialogue = String::new();
    for turn in turns {
        let (who, limit) = if turn.role == "user" { ("Usuário", 2000) } else { ("Ayla", 400) };
        let text: String = turn.content.chars().take(limit).collect();
        dialogue.push_str(&format!("{who}: {text}\n"));
    }
    let existing = if related.is_empty() {
        "(nenhuma)".to_string()
    } else {
        related
            .iter()
            .enumerate()
            .map(|(i, f)| format!("[{i}] {}", f.content))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let instructions = "Você mantém a memória de longo prazo de uma assistente pessoal. Leia a conversa e devolva somente JSON.\n\
Regras:\n\
- Guarde apenas informações duradouras sobre o usuário ditas por ele: preferências, fatos pessoais, pessoas próximas, rotinas, objetivos e projetos. Ignore pedidos pontuais, perguntas, assuntos gerais e o que a Ayla disse.\n\
- Não guarde nada temporário ou que muda com o tempo: saldos, valores de faturas, gastos do mês, preços, cotações, compromissos de hoje ou estados do momento. Também não guarde dados vindos dos órgãos da Ayla (finanças da Moneta, trechos de arquivos): eles continuam disponíveis na fonte.\n\
- Se o usuário pedir para lembrar algo, guarde. Se pedir para esquecer, indique em \"esquecer\" o número da memória existente correspondente.\n\
- Escreva cada memória como frase curta em terceira pessoa, começando por \"O usuário\".\n\
- Se uma informação nova atualiza ou contradiz uma memória existente, use \"substitui\" com o número dela. Não repita memórias existentes sem mudança.\n\
- privacidade: \"sensivel\" para saúde, finanças pessoais, documentos, endereço, senhas, relações íntimas, crenças ou dados de terceiros; caso contrário \"duvidoso\".\n\
- No máximo 5 memórias. Se não houver nada a guardar, devolva listas vazias.";
    vec![
        json!({"role":"system","content":instructions}),
        json!({"role":"user","content":format!("Memórias existentes:\n{existing}\n\nConversa:\n{dialogue}")}),
    ]
}
pub fn extraction_schema() -> Value {
    json!({
        "type":"object",
        "properties":{
            "lembrar":{"type":"array","maxItems":5,"items":{"type":"object","properties":{
                "texto":{"type":"string","minLength":5,"maxLength":300},
                "tipo":{"type":"string","enum":["preferencia","fato","pessoa","rotina","objetivo","projeto"]},
                "privacidade":{"type":"string","enum":["sensivel","duvidoso"]},
                "substitui":{"type":["integer","null"]}
            },"required":["texto","tipo","privacidade","substitui"]}},
            "esquecer":{"type":"array","maxItems":5,"items":{"type":"integer"}}
        },
        "required":["lembrar","esquecer"]
    })
}

#[derive(Debug, PartialEq)]
pub struct NewFact {
    pub content: String,
    pub kind: String,
    pub privacy: Privacy,
    /// Id da memória existente que esta substitui.
    pub replaces: Option<String>,
}
#[derive(Debug, Default, PartialEq)]
pub struct Extraction {
    pub remember: Vec<NewFact>,
    pub forget: Vec<String>,
}
/// Interpreta a saída do modelo; índices fora da lista de relacionadas são ignorados.
/// A privacidade nunca é marcada como pública por classificação automática.
/// Memória automática que só vale por um tempo (valores em dinheiro, "hoje", "em aberto"…).
/// Fica de fora mesmo se o modelo de extração a propuser; pedidos explícitos não passam por aqui.
pub fn temporary(content: &str) -> bool {
    let lower = content.to_lowercase();
    let money = ["r$", "us$", "€", "£", " usd", " brl", " reais", " real "].iter().any(|m| lower.contains(m))
        || lower.as_bytes().windows(4).any(|w| w[0].is_ascii_digit() && w[1] == b',' && w[2].is_ascii_digit() && w[3].is_ascii_digit());
    let moment = [
        "hoje", "amanhã", "amanha", "ontem", "neste momento", "no momento", "este mês", "esse mês", "neste mês",
        "esta semana", "nesta semana", "em aberto", "saldo atual", "atualmente com",
    ]
    .iter()
    .any(|w| lower.contains(w));
    money || moment
}

pub fn parse_extraction(raw: &str, related: &[Fact]) -> Result<Extraction, String> {
    let value: Value = serde_json::from_str(raw.trim()).map_err(|_| "Extração inválida")?;
    let index = |v: &Value| v.as_u64().and_then(|i| related.get(i as usize)).map(|f| f.id.clone());
    let mut out = Extraction::default();
    for item in value["lembrar"].as_array().into_iter().flatten().take(5) {
        let content = item["texto"].as_str().unwrap_or("").trim();
        if content.chars().count() < 5 || content.chars().count() > 300 {
            continue;
        }
        out.remember.push(NewFact {
            content: content.to_string(),
            kind: item["tipo"].as_str().unwrap_or("fato").to_string(),
            privacy: if item["privacidade"] == "sensivel" { Privacy::Sensitive } else { Privacy::Uncertain },
            replaces: index(&item["substitui"]),
        });
    }
    out.forget = value["esquecer"].as_array().into_iter().flatten().filter_map(index).collect();
    out.forget.dedup();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fact(id: &str, content: &str) -> Fact {
        Fact { id: id.into(), content: content.into(), kind: "fato".into(), privacy: Privacy::Uncertain, origin: "auto".into(), conversation_id: None, created_at: 0, updated_at: 0, project_id: None }
    }
    #[test]
    fn referencias_so_publicas_iguais_e_no_escopo() {
        let mut public = fact("a", "O usuário prefere Python.");
        public.privacy = Privacy::Public;
        let mut other_project = public.clone();
        other_project.id = "b".into();
        other_project.project_id = Some("outro".into());
        let uncertain = fact("c", "O usuário mora em Recife.");
        let facts = vec![public.clone(), other_project, uncertain];
        let r = |id: &str, content: &str| Reference { id: id.into(), content: content.into() };
        assert!(check_references(&[r("a", "O usuário prefere Python.")], &facts, None).is_ok());
        assert!(check_references(&[r("a", "O usuário prefere Python.")], &facts, Some("p")).is_ok(), "geral vale no projeto");
        assert!(check_references(&[r("a", "editada")], &facts, None).is_err(), "mudou depois da revisão");
        assert!(check_references(&[r("c", "O usuário mora em Recife.")], &facts, None).is_err(), "não pública");
        assert!(check_references(&[r("b", "O usuário prefere Python.")], &facts, Some("p")).is_err(), "outro projeto");
        assert!(check_references(&[r("x", "?")], &facts, None).is_err());
        let many: Vec<Reference> = (0..6).map(|_| r("a", "O usuário prefere Python.")).collect();
        assert!(check_references(&many, &facts, None).is_err());
        assert!(reference_block(&[r("a", "O usuário prefere Python.")]).unwrap().ends_with("\n- O usuário prefere Python."));
        assert!(reference_block(&[]).is_none());
    }
    #[test]
    fn memorias_temporarias_ficam_de_fora() {
        assert!(temporary("O usuário tem uma fatura de cartão de crédito em aberto no valor de R$ 2.730,75."));
        assert!(temporary("O usuário gastou 350,00 no mercado."));
        assert!(temporary("O usuário tem reunião hoje às 15h."));
        assert!(!temporary("O usuário prefere respostas curtas."));
        assert!(!temporary("O usuário usa o cartão Nubank para compras online."));
        assert!(!temporary("O usuário mora em Recife desde 2019."));
    }
    #[test]
    fn vectors_round_trip_and_reject_corruption() {
        let v = vec![0.5f32, -1.25, 3.0];
        assert_eq!(decode(&encode(&v)).unwrap(), v);
        assert!(decode(&[1, 2, 3]).is_none() && decode(&[]).is_none());
        assert!(cosine(&v, &[1.0]).is_none() && cosine(&[0.0, 0.0], &[0.0, 0.0]).is_none());
        assert!((cosine(&v, &v).unwrap() - 1.0).abs() < 1e-9);
    }
    #[test]
    fn fts_query_quotes_terms_and_drops_noise() {
        assert_eq!(fts_query("Qual o nome da minha mesa de RPG?").unwrap(), "\"mesa\" OR \"nome\" OR \"rpg\"");
        assert!(fts_query("o que é isso?").is_none());
        // Aspas e operadores do usuário não chegam à sintaxe FTS.
        assert_eq!(fts_query("mel\" OR x NEAR(").unwrap(), "\"mel\" OR \"near\"");
    }
    #[test]
    fn fusion_keeps_vector_hits_ahead_of_text_only_hits() {
        let vector = vec![("a".to_string(), 0.7), ("b".to_string(), 0.5)];
        let text = vec!["c".to_string(), "b".to_string()];
        assert_eq!(fuse(&vector, &text, 5), vec!["b", "a", "c"]);
        assert_eq!(fuse(&vector, &text, 1), vec!["b"]);
        assert_eq!(fuse(&[], &text, 5), vec!["c", "b"]);
    }
    #[test]
    fn extraction_parsing_maps_indices_and_never_marks_public() {
        let related = vec![fact("x", "O usuário mora em Fortaleza."), fact("y", "O usuário tem uma gata.")];
        let raw = r#"{"lembrar":[{"texto":"O usuário mora em Recife.","tipo":"fato","privacidade":"duvidoso","substitui":0},
            {"texto":"O usuário faz tratamento de asma.","tipo":"fato","privacidade":"sensivel","substitui":null},
            {"texto":"ok","tipo":"fato","privacidade":"duvidoso","substitui":null},
            {"texto":"O usuário gosta de chá.","tipo":"preferencia","privacidade":"publico","substitui":9}],
            "esquecer":[1,1,7]}"#;
        let out = parse_extraction(raw, &related).unwrap();
        assert_eq!(out.remember.len(), 3);
        assert_eq!(out.remember[0].replaces.as_deref(), Some("x"));
        assert_eq!(out.remember[1].privacy, Privacy::Sensitive);
        assert_eq!((out.remember[2].privacy, out.remember[2].replaces.clone()), (Privacy::Uncertain, None));
        assert_eq!(out.forget, vec!["y".to_string()]);
        assert!(parse_extraction("não é json", &related).is_err());
        assert_eq!(parse_extraction(r#"{"lembrar":[],"esquecer":[]}"#, &[]).unwrap(), Extraction::default());
    }
    #[test]
    fn explicit_commands_and_context_block() {
        assert!(explicit_command("Ayla, lembre que eu odeio coentro"));
        assert!(explicit_command("Pode esquecer meu endereço"));
        assert!(!explicit_command("Você lembra do filme?"));
        assert!(context_block(&[]).is_none());
        assert!(context_block(&[fact("a", "O usuário é vegetariano.")]).unwrap().contains("- O usuário é vegetariano."));
    }
}
