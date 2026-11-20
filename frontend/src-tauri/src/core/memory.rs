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
/// Consulta FTS5 segura: só palavras relevantes, cada uma entre aspas.
pub fn fts_query(text: &str) -> Option<String> {
    let mut terms: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .map(|w| w.to_lowercase())
        .filter(|w| w.chars().count() >= 3 && !STOPWORDS.contains(&w.as_str()))
        .collect();
    terms.sort();
    terms.dedup();
    terms.truncate(12);
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
