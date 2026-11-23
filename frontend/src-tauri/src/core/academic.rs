//! Pesquisa acadêmica em bases abertas (OpenAlex, Crossref e arXiv). O Google Acadêmico não tem
//! API e bloqueia automação: entra como link com a mesma busca. Só os termos saem do computador,
//! num pedido explícito; a resposta é escrita localmente, citando as referências encontradas.
use super::bases::fold;
use serde::Serialize;
use serde_json::{json, Value};

const TRIGGERS: &[&str] = &[
    "artigo cientifico", "artigos cientificos", "artigo academico", "artigos academicos", "paper", "papers", "pesquisa academica",
    "pesquisas academicas", "estudo cientifico", "estudos cientificos", "revisao de literatura", "revisao bibliografica",
    "referencias bibliograficas", "referencia bibliografica", "google academico", "google scholar", "literatura cientifica",
    "publicacoes cientificas", "publicacao cientifica", "bibliografia", "producao cientifica", "evidencias cientificas",
];

/// Pedido explícito de pesquisa acadêmica.
pub fn request(message: &str) -> bool {
    let text = fold(message);
    TRIGGERS.iter().any(|t| text.contains(&format!(" {t} ")))
}

/// Termos pelas regras: o que vem depois de "sobre", "de" ou "acerca de" no trecho do pedido.
pub fn terms(message: &str) -> String {
    let text = fold(message);
    let mut rest = text.as_str();
    for t in TRIGGERS {
        if let Some(at) = rest.find(&format!(" {t} ")) { rest = &rest[at + t.len() + 1..]; }
    }
    let words: Vec<&str> = rest.split_whitespace().collect();
    let skip = ["sobre", "de", "do", "da", "dos", "das", "acerca", "a", "respeito", "que", "falam", "falem", "tratam", "em", "no", "na"];
    let stop = ["para", "pra", "no google", "e", "me", "por"];
    let out: Vec<&str> = words.iter().skip_while(|w| skip.contains(w)).take_while(|w| !stop.contains(w)).copied().collect();
    if out.is_empty() { text.trim().to_string() } else { out.join(" ") }
}

pub fn terms_request(message: &str, rules: &str) -> Vec<Value> {
    vec![
        json!({"role":"system","content":"Extraia o tema de uma pesquisa acadêmica. Responda só com JSON {\"pt\": \"termos em português\", \"en\": \"terms in English\"}, 2 a 6 palavras-chave cada, sem palavras como artigo, paper, pesquisa, estudo."}),
        json!({"role":"user","content":format!("Pedido: {message}\nSugestão pelas regras: {rules}")}),
    ]
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Work {
    pub title: String,
    pub authors: Vec<String>,
    pub year: Option<i64>,
    pub venue: String,
    pub doi: Option<String>,
    pub url: String,
    pub pdf: Option<String>,
    pub cited_by: Option<u64>,
    pub summary: String,
    pub source: &'static str,
}

fn clip(text: &str, limit: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > limit { format!("{}…", text.chars().take(limit).collect::<String>()) } else { text }
}

fn strip_tags(text: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in text.chars() {
        match c { '<' => inside = true, '>' => inside = false, c if !inside => out.push(c), _ => {} }
    }
    out
}

/// Resumo do OpenAlex: vem como índice invertido (palavra → posições).
pub fn abstract_from_index(index: &Value) -> String {
    let Some(map) = index.as_object() else { return String::new() };
    let mut words: Vec<(u64, &str)> = Vec::new();
    for (word, positions) in map {
        for p in positions.as_array().into_iter().flatten().filter_map(Value::as_u64) { words.push((p, word)); }
    }
    words.sort();
    words.into_iter().map(|(_, w)| w).collect::<Vec<_>>().join(" ")
}

pub fn parse_openalex(data: &Value) -> Vec<Work> {
    data["results"].as_array().into_iter().flatten().filter_map(|w| {
        let title = w["display_name"].as_str()?.trim().to_string();
        let doi = w["doi"].as_str().map(|d| d.trim_start_matches("https://doi.org/").to_string());
        Some(Work {
            authors: w["authorships"].as_array().into_iter().flatten().filter_map(|a| a["author"]["display_name"].as_str().map(str::to_string)).take(3).collect(),
            year: w["publication_year"].as_i64(), venue: w["primary_location"]["source"]["display_name"].as_str().unwrap_or("").to_string(),
            url: doi.as_ref().map(|d| format!("https://doi.org/{d}")).or_else(|| w["primary_location"]["landing_page_url"].as_str().map(str::to_string)).unwrap_or_else(|| w["id"].as_str().unwrap_or("").to_string()),
            pdf: w["open_access"]["oa_url"].as_str().map(str::to_string), cited_by: w["cited_by_count"].as_u64(),
            summary: clip(&abstract_from_index(&w["abstract_inverted_index"]), 600), doi, title, source: "OpenAlex",
        })
    }).collect()
}

pub fn parse_crossref(data: &Value) -> Vec<Work> {
    data["message"]["items"].as_array().into_iter().flatten().filter_map(|w| {
        let title = w["title"][0].as_str()?.trim().to_string();
        let doi = w["DOI"].as_str().map(str::to_string);
        Some(Work {
            authors: w["author"].as_array().into_iter().flatten().filter_map(|a| Some(format!("{} {}", a["given"].as_str().unwrap_or(""), a["family"].as_str()?).trim().to_string())).take(3).collect(),
            year: w["issued"]["date-parts"][0][0].as_i64(), venue: w["container-title"][0].as_str().unwrap_or("").to_string(),
            url: doi.as_ref().map(|d| format!("https://doi.org/{d}")).unwrap_or_else(|| w["URL"].as_str().unwrap_or("").to_string()),
            pdf: None, cited_by: w["is-referenced-by-count"].as_u64(), summary: clip(&strip_tags(w["abstract"].as_str().unwrap_or("")), 600),
            doi, title, source: "Crossref",
        })
    }).collect()
}

fn tag<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}");
    let start = text.find(&open)?;
    let body = text[start..].find('>')? + start + 1;
    let end = text[body..].find(&format!("</{name}>"))? + body;
    Some(&text[body..end])
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&")
}

/// Atom do arXiv, lido entrada por entrada.
pub fn parse_arxiv(xml: &str) -> Vec<Work> {
    xml.split("<entry>").skip(1).filter_map(|entry| {
        let title = unescape(&clip(tag(entry, "title")?, 300));
        let url = tag(entry, "id")?.trim().to_string();
        let authors = entry.split("<author>").skip(1).filter_map(|a| tag(a, "name").map(|n| unescape(n.trim()))).take(3).collect();
        let year = tag(entry, "published").and_then(|p| p.get(0..4)?.parse().ok());
        let pdf = entry.split("<link").find(|l| l.contains("title=\"pdf\"")).and_then(|l| l.split("href=\"").nth(1)?.split('"').next()).map(str::to_string);
        let doi = tag(entry, "arxiv:doi").map(|d| d.trim().to_string());
        Some(Work { title, authors, year, venue: "arXiv".into(), doi, url, pdf, cited_by: None, summary: unescape(&clip(tag(entry, "summary").unwrap_or(""), 600)), source: "arXiv" })
    }).collect()
}

fn encode(text: &str) -> String {
    text.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// Mesma busca no Google Acadêmico, para abrir no navegador.
pub fn scholar_url(query: &str) -> String {
    format!("https://scholar.google.com/scholar?q={}", encode(query))
}

/// Busca em paralelo; termos em português no OpenAlex (pega SciELO e revistas nacionais) e em
/// inglês em todas. Junta sem repetir (DOI ou título) e devolve as fontes que falharam.
/// Fontes da pesquisa (id, nome); "scholar" é só link.
pub const SOURCES: &[(&str, &str)] = &[("openalex", "OpenAlex"), ("crossref", "Crossref"), ("arxiv", "arXiv"), ("scholar", "Google Acadêmico")];

/// `only` vazio usa todas; senão, só as escolhidas na barra.
pub async fn search(client: &reqwest::Client, pt: &str, en: &str, only: &[String]) -> (Vec<Work>, Vec<&'static str>) {
    let wants = |id: &str| only.is_empty() || only.iter().any(|o| o == id);
    let select = "id,doi,display_name,publication_year,authorships,primary_location,open_access,cited_by_count,abstract_inverted_index";
    let openalex = |q: String| {
        let client = client.clone();
        let on = wants("openalex");
        async move { if !on { return None; } client.get(format!("https://api.openalex.org/works?search={}&per-page=5&select={select}", encode(&q))).send().await.ok()?.error_for_status().ok()?.json::<Value>().await.ok().map(|d| parse_openalex(&d)) }
    };
    let crossref = async { if !wants("crossref") { return None; } client.get(format!("https://api.crossref.org/works?query={}&rows=4&select=DOI,title,author,issued,container-title,abstract,is-referenced-by-count,URL", encode(en))).send().await.ok()?.error_for_status().ok()?.json::<Value>().await.ok().map(|d| parse_crossref(&d)) };
    let arxiv = async { if !wants("arxiv") { return None; } client.get(format!("https://export.arxiv.org/api/query?search_query=all:{}&max_results=3", encode(en))).send().await.ok()?.error_for_status().ok()?.text().await.ok().map(|x| parse_arxiv(&x)) };
    let (a, b, c, d) = tokio::join!(openalex(pt.to_string()), openalex(en.to_string()), crossref, arxiv);
    let mut failed = Vec::new();
    if a.is_none() && b.is_none() && wants("openalex") { failed.push("OpenAlex"); }
    if c.is_none() && wants("crossref") { failed.push("Crossref"); }
    if d.is_none() && wants("arxiv") { failed.push("arXiv"); }
    let lists: Vec<Vec<Work>> = [a, b, c, d].into_iter().flatten().collect();
    let mut out: Vec<Work> = Vec::new();
    let key = |w: &Work| w.doi.clone().map(|d| d.to_lowercase()).unwrap_or_else(|| fold(&w.title));
    for i in 0..5 {
        for list in &lists {
            if let Some(work) = list.get(i) {
                if !out.iter().any(|o| key(o) == key(work)) { out.push(work.clone()); }
            }
        }
    }
    out.truncate(10);
    (out, failed)
}

fn line(n: usize, w: &Work) -> String {
    let authors = if w.authors.is_empty() { String::new() } else { format!("{}. ", w.authors.join(", ")) };
    let year = w.year.map(|y| format!(" ({y})")).unwrap_or_default();
    let venue = if w.venue.is_empty() { String::new() } else { format!(" {}.", w.venue) };
    format!("[{n}] {authors}{}{year}.{venue} {}", w.title, w.url)
}

/// Bloco para o prompt local: referências numeradas, com resumos, e a instrução de citar.
pub fn context_block(works: &[Work], query: &str) -> String {
    let mut out = format!("\n\nResultados de pesquisa acadêmica para \"{query}\" (OpenAlex, Crossref e arXiv, consultados agora):\n");
    for (i, w) in works.iter().enumerate() {
        out.push_str(&line(i + 1, w));
        if let Some(c) = w.cited_by { out.push_str(&format!(" · citado {c} vezes")); }
        if let Some(pdf) = &w.pdf { out.push_str(&format!(" · PDF aberto: {pdf}")); }
        if !w.summary.is_empty() { out.push_str(&format!("\nResumo: {}", w.summary)); }
        out.push('\n');
    }
    out.push_str(&format!("Responda em português com base só nestes trabalhos, citando-os como [n]. Não invente autores, anos nem resultados. Termine com a seção \"Referências\" com os itens usados e seus links, e com o link para continuar no Google Acadêmico: {}", scholar_url(query)));
    out
}

/// Resposta sem modelo local: a lista das referências encontradas.
pub fn plain_reply(works: &[Work], query: &str, failed: &[&str]) -> String {
    let mut out = if works.is_empty() { format!("Não encontrei trabalhos acadêmicos para \"{query}\".") } else { format!("Trabalhos acadêmicos sobre \"{query}\":\n\n") };
    for (i, w) in works.iter().enumerate() {
        out.push_str(&format!("- {}", line(i + 1, w)));
        if let Some(pdf) = &w.pdf { out.push_str(&format!(" · [PDF aberto]({pdf})")); }
        out.push('\n');
    }
    out.push_str(&format!("\n[Continuar no Google Acadêmico]({})", scholar_url(query)));
    if !failed.is_empty() { out.push_str(&format!("\n\n{} não respondeu agora.", failed.join(", "))); }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconhece_pedidos_e_le_as_fontes() {
        assert!(request("me traz artigos científicos sobre reforma tributária e consumo"));
        assert!(request("quero papers sobre transformers"));
        assert!(!request("escreve um artigo para o meu blog"));
        assert_eq!(terms("me traz artigos científicos sobre reforma tributária e consumo"), "reforma tributaria");
        assert_eq!(terms("quero papers sobre visão computacional para agricultura"), "visao computacional");
        let index = json!({"Reforma":[0],"tributária":[1],"no":[2],"Brasil":[3]});
        assert_eq!(abstract_from_index(&index), "Reforma tributária no Brasil");
        let openalex = json!({"results":[{"id":"https://openalex.org/W1","doi":"https://doi.org/10.1/x","display_name":"Título","publication_year":2024,
            "authorships":[{"author":{"display_name":"Ana Souza"}}],"primary_location":{"source":{"display_name":"Revista X"}},"open_access":{"oa_url":"https://x/pdf"},"cited_by_count":3,"abstract_inverted_index":index}]});
        let work = &parse_openalex(&openalex)[0];
        assert_eq!((work.url.as_str(), work.doi.as_deref(), work.year), ("https://doi.org/10.1/x", Some("10.1/x"), Some(2024)));
        let crossref = json!({"message":{"items":[{"DOI":"10.2/y","title":["Outro"],"author":[{"given":"João","family":"Lima"}],"issued":{"date-parts":[[2021,5]]},"abstract":"<jats:p>Texto</jats:p>"}]}});
        let work = &parse_crossref(&crossref)[0];
        assert_eq!((work.authors[0].as_str(), work.summary.as_str()), ("João Lima", "Texto"));
        let xml = r#"<feed><entry><id>http://arxiv.org/abs/1706.03762v7</id><published>2017-06-12T17:57:34Z</published><title>Attention Is
 All You Need</title><summary>We propose &amp; test.</summary><author><name>Ashish Vaswani</name></author><link title="pdf" href="https://arxiv.org/pdf/1706.03762v7" rel="related"/></entry></feed>"#;
        let work = &parse_arxiv(xml)[0];
        assert_eq!((work.title.as_str(), work.year, work.pdf.as_deref(), work.summary.as_str()), ("Attention Is All You Need", Some(2017), Some("https://arxiv.org/pdf/1706.03762v7"), "We propose & test."));
        let block = context_block(&[work.clone()], "atenção");
        assert!(block.contains("[1] Ashish Vaswani. Attention Is All You Need (2017). arXiv.") && block.contains("scholar.google.com/scholar?q=aten%C3%A7%C3%A3o"));
        assert!(plain_reply(&[], "x", &["arXiv"]).contains("arXiv não respondeu"));
    }

    /// Real (rede): `cargo test --lib -- --ignored academico_real --nocapture`
    #[test]
    #[ignore]
    fn academico_real() {
        let client = reqwest::Client::builder().user_agent("Ayla/0.1 (assistente local)").build().unwrap();
        let (works, failed) = tauri::async_runtime::block_on(search(&client, "reforma tributária consumo", "tax reform consumption", &[]));
        for w in &works { eprintln!("{} · {:?} · {} · {}", w.source, w.year, w.title, w.url); }
        assert!(works.len() >= 5, "{failed:?}");
    }
}
