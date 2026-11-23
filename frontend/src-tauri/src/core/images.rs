//! Busca de imagens: referências, imagens para projetos e wallpapers. Só os termos da busca saem
//! do computador, e só num pedido explícito. Cada resultado traz fonte, autor e licença: as de
//! Unsplash, Pexels e Pixabay são livres para usar (apresentações, mockups, uso comercial); as do
//! Wallhaven têm direitos dos autores e servem de referência. Pinterest, Cosmos, same.energy e
//! Are.na não têm busca pública por API: entram como links para abrir no navegador.
use super::bases::fold;
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// Papel de parede (alta resolução, paisagem).
    Wallpaper,
    /// Para usar em apresentações, mockups e peças: só licenças livres.
    Free,
    /// Referências e inspiração: todas as fontes.
    Reference,
}

const NOUNS: &[&str] = &[
    "imagem", "imagens", "foto", "fotos", "fotografia", "fotografias", "wallpaper", "wallpapers", "papel de parede", "papeis de parede",
    "referencia visual", "referencias visuais", "moodboard", "mood board", "ilustracao", "ilustracoes", "inspiracao visual", "inspiracoes visuais",
];
const VERBS: &[&str] = &[
    "busca", "buscar", "busque", "procura", "procurar", "procure", "encontra", "encontrar", "encontre", "acha", "achar", "ache", "pesquisa",
    "pesquisar", "pesquise", "mostra", "mostre", "quero", "preciso", "sugere", "sugira", "traz", "traga", "separa", "separe", "junta", "junte",
    "retorna", "retorne", "retornar", "manda", "mande", "mandar", "envia", "envie", "enviar", "passa", "passe", "lista", "liste", "listar",
    "indica", "indique", "recomenda", "recomende", "gostaria", "baixa", "baixe", "consegue", "arruma", "arrume", "arranja", "arranje",
];
/// Pedidos para criar uma imagem vão para a geração, não para a busca.
const CREATE: &[&str] = &["cria", "crie", "criar", "gera", "gere", "gerar", "desenha", "desenhe", "desenhar", "faz uma imagem", "faca uma imagem"];
const FREE: &[&str] = &["apresentacao", "apresentacoes", "slide", "slides", "mockup", "mockups", "livre", "livres", "sem direitos", "royalty", "uso comercial", "copyright", "gratuita", "gratuitas", "banco de imagens"];

fn has(text: &str, terms: &[&str]) -> bool {
    terms.iter().any(|t| text.contains(&format!(" {t} ")))
}

/// Pedido explícito de busca de imagens.
pub fn request(message: &str) -> Option<Kind> {
    let text = fold(message);
    if !has(&text, NOUNS) || has(&text, CREATE) {
        return None;
    }
    let starts = NOUNS.iter().any(|n| text.trim_start().starts_with(n));
    if !has(&text, VERBS) && !starts && !count(message).is_some_and(|n| n > 1) {
        return None;
    }
    Some(if has(&text, &["wallpaper", "wallpapers", "papel de parede", "papeis de parede"]) { Kind::Wallpaper }
        else if has(&text, FREE) { Kind::Free } else { Kind::Reference })
}

/// Termos da busca pelas regras: o que vem depois do tipo de imagem pedido, até "para"/"pra"/"que".
pub fn terms(message: &str) -> String {
    let text = fold(message);
    let mut best = None;
    for noun in NOUNS {
        if let Some(at) = text.find(&format!(" {noun} ")) {
            let end = at + noun.len() + 1;
            if best.is_none_or(|b| end > b) { best = Some(end); }
        }
    }
    let rest = best.map_or(text.as_str(), |at| &text[at..]);
    let words: Vec<&str> = rest.split_whitespace().collect();
    let skip = ["de", "do", "da", "dos", "das", "sobre", "com", "em", "no", "na", "um", "uma", "algumas", "alguns", "boas", "bons", "bonitas", "bonitos", "legais", "livre", "livres", "gratuita", "gratuitas", "gratis", "boa", "bom"];
    let stop = ["para", "pra", "que", "porque", "pro", "livre", "livres", "sem", "em alta"];
    let mut out = Vec::new();
    for word in words.iter().skip_while(|w| skip.contains(w)) {
        if stop.contains(word) || (has(&format!(" {word} "), FREE) && !out.is_empty()) { break; }
        out.push(*word);
    }
    let joined = out.join(" ");
    if joined.is_empty() { text.split_whitespace().filter(|w| !VERBS.contains(w) && !NOUNS.contains(w)).collect::<Vec<_>>().join(" ") } else { joined }
}

/// Pedido ao modelo local: termos curtos em português e em inglês (as fontes buscam melhor em inglês).
pub fn terms_request(message: &str, rules: &str) -> Vec<Value> {
    vec![
        json!({"role":"system","content":"Extraia o assunto de uma busca de imagens. Responda só com JSON {\"pt\": \"termos em português\", \"en\": \"terms in English\"}, 1 a 5 palavras cada, sem palavras como imagem, foto, wallpaper, apresentação, livre."}),
        json!({"role":"user","content":format!("Pedido: {message}\nSugestão pelas regras: {rules}")}),
    ]
}
pub fn terms_schema() -> Value {
    json!({"type":"object","properties":{"pt":{"type":"string"},"en":{"type":"string"}},"required":["pt","en"],"additionalProperties":false})
}
pub fn parse_terms(raw: &str) -> Option<(String, String)> {
    let value: Value = serde_json::from_str(raw.trim()).ok()?;
    let clean = |v: &Value| v.as_str().map(|s| clean_terms(s.trim()).chars().take(100).collect::<String>()).filter(|s| !s.is_empty());
    Some((clean(&value["pt"])?, clean(&value["en"])?))
}

/// Palavras que só dizem o tipo ou elogiam ("wallpapers", "imagens", "legais", "4k") atrapalham as
/// fontes, que buscam por etiquetas: saem dos termos (se sobrar algo).
const GENERIC: &[&str] = &[
    "wallpaper", "wallpapers", "papel", "papeis", "parede", "imagem", "imagens", "image", "images", "foto", "fotos", "photo", "photos",
    "picture", "pictures", "background", "backgrounds", "legal", "legais", "cool", "nice", "bonito", "bonitos", "bonita", "bonitas",
    "lindo", "lindos", "linda", "lindas", "beautiful", "hd", "4k", "uns", "umas", "some",
];
pub fn clean_terms(terms: &str) -> String {
    let kept: Vec<&str> = terms.split_whitespace()
        .filter(|w| { let f = fold(w); let f = f.trim(); !GENERIC.contains(&f) && f.parse::<u32>().is_err() })
        .collect();
    let leading = ["de", "do", "da", "dos", "das", "of", "para", "pra", "com", "sobre"];
    let kept: Vec<&str> = kept.into_iter().skip_while(|w| leading.contains(&fold(w).trim())).collect();
    if kept.is_empty() { terms.to_string() } else { kept.join(" ") }
}

/// Quantidade pedida ("uns 5 wallpapers", "três fotos"), entre 1 e 24.
pub fn count(message: &str) -> Option<usize> {
    let words: Vec<String> = fold(message).split_whitespace().map(str::to_string).collect();
    let named = [("um", 1), ("uma", 1), ("dois", 2), ("duas", 2), ("tres", 3), ("quatro", 4), ("cinco", 5), ("seis", 6), ("sete", 7), ("oito", 8), ("nove", 9), ("dez", 10), ("doze", 12), ("quinze", 15), ("vinte", 20)];
    let nouns = ["imagem", "imagens", "foto", "fotos", "fotografia", "fotografias", "wallpaper", "wallpapers", "papel", "papeis", "referencia", "referencias", "ilustracao", "ilustracoes", "opcoes", "exemplos"];
    words.iter().enumerate().find_map(|(i, w)| {
        let n = w.parse::<usize>().ok().or_else(|| named.iter().find(|(name, _)| name == w).map(|(_, n)| *n))?;
        words[i + 1..(i + 4).min(words.len())].iter().any(|next| nouns.contains(&next.as_str())).then_some(n.clamp(1, 24))
    })
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Item {
    pub provider: &'static str,
    pub id: String,
    pub thumb: String,
    pub full: String,
    pub page: String,
    pub author: String,
    pub author_url: String,
    /// "livre" (pode usar em apresentações, mockups e peças) ou "referencia" (direitos dos autores).
    pub license: &'static str,
    pub license_name: &'static str,
    pub license_url: &'static str,
    pub width: u64,
    pub height: u64,
    /// Unsplash: endereço que registra o download, chamado ao salvar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_ping: Option<String>,
}

#[derive(Default, Clone)]
pub struct Keys {
    pub unsplash: Option<String>,
    pub pexels: Option<String>,
    pub pixabay: Option<String>,
}

pub const PROVIDERS: &[&str] = &["unsplash", "pexels", "pixabay"];
/// Identificação nos links do Unsplash (pedido das diretrizes da API).
const UTM: &str = "utm_source=ayla&utm_medium=referral";

fn with_utm(url: &str) -> String {
    if url.is_empty() { return String::new(); }
    format!("{url}{}{UTM}", if url.contains('?') { '&' } else { '?' })
}
fn s(v: &Value) -> String { v.as_str().unwrap_or("").to_string() }

pub fn parse_unsplash(data: &Value) -> Vec<Item> {
    data["results"].as_array().into_iter().flatten().filter_map(|p| Some(Item {
        provider: "unsplash", id: p["id"].as_str()?.to_string(), thumb: s(&p["urls"]["small"]), full: s(&p["urls"]["full"]),
        page: with_utm(&s(&p["links"]["html"])), author: s(&p["user"]["name"]), author_url: with_utm(&s(&p["user"]["links"]["html"])),
        license: "livre", license_name: "Licença Unsplash", license_url: "https://unsplash.com/license",
        width: p["width"].as_u64().unwrap_or(0), height: p["height"].as_u64().unwrap_or(0), download_ping: p["links"]["download_location"].as_str().map(str::to_string),
    })).collect()
}
pub fn parse_pexels(data: &Value) -> Vec<Item> {
    data["photos"].as_array().into_iter().flatten().filter_map(|p| Some(Item {
        provider: "pexels", id: p["id"].as_u64()?.to_string(), thumb: s(&p["src"]["medium"]), full: s(&p["src"]["original"]), page: s(&p["url"]),
        author: s(&p["photographer"]), author_url: s(&p["photographer_url"]), license: "livre", license_name: "Licença Pexels", license_url: "https://www.pexels.com/license/",
        width: p["width"].as_u64().unwrap_or(0), height: p["height"].as_u64().unwrap_or(0), download_ping: None,
    })).collect()
}
pub fn parse_pixabay(data: &Value) -> Vec<Item> {
    data["hits"].as_array().into_iter().flatten().filter_map(|p| Some(Item {
        provider: "pixabay", id: p["id"].as_u64()?.to_string(), thumb: s(&p["webformatURL"]), full: s(&p["largeImageURL"]), page: s(&p["pageURL"]),
        author: s(&p["user"]), author_url: format!("https://pixabay.com/users/{}-{}/", s(&p["user"]), p["user_id"].as_u64().unwrap_or(0)),
        license: "livre", license_name: "Licença de conteúdo Pixabay", license_url: "https://pixabay.com/service/license-summary/",
        width: p["imageWidth"].as_u64().unwrap_or(0), height: p["imageHeight"].as_u64().unwrap_or(0), download_ping: None,
    })).collect()
}
pub fn parse_wallhaven(data: &Value) -> Vec<Item> {
    data["data"].as_array().into_iter().flatten().filter(|p| p["purity"] == "sfw").filter_map(|p| {
        let (w, h) = p["resolution"].as_str().and_then(|r| r.split_once('x')).map_or((0, 0), |(w, h)| (w.parse().unwrap_or(0), h.parse().unwrap_or(0)));
        Some(Item {
            provider: "wallhaven", id: p["id"].as_str()?.to_string(), thumb: s(&p["thumbs"]["large"]), full: s(&p["path"]), page: s(&p["url"]),
            author: String::new(), author_url: s(&p["source"]), license: "referencia", license_name: "Direitos dos autores (Wallhaven)",
            license_url: "https://wallhaven.cc/about", width: w, height: h, download_ping: None,
        })
    }).collect()
}

fn encode(text: &str) -> String {
    text.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// Fontes consultadas para cada tipo de pedido.
pub fn sources(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::Free => &["unsplash", "pexels", "pixabay"],
        Kind::Wallpaper => &["wallhaven", "unsplash", "pexels"],
        Kind::Reference => &["unsplash", "pexels", "pixabay", "wallhaven"],
    }
}

/// Busca em paralelo nas fontes com chave (e no Wallhaven, que não pede). Devolve os itens
/// intercalados entre as fontes e as fontes que falharam.
/// Fontes com API usadas no pedido: as do tipo ou, se a pessoa escolheu fontes na barra, só elas.
pub fn chosen(kind: Kind, only: &[String]) -> Vec<&'static str> {
    let all: &[&str] = &["unsplash", "pexels", "pixabay", "wallhaven"];
    if only.is_empty() { sources(kind).to_vec() } else { all.iter().copied().filter(|p| only.iter().any(|o| o == p)).collect() }
}

pub async fn search(client: &reqwest::Client, kind: Kind, query: &str, keys: &Keys, providers: &[&'static str]) -> (Vec<Item>, Vec<String>) {
    let q = encode(query);
    let landscape = kind == Kind::Wallpaper;
    let run = |provider: &'static str| {
        let client = client.clone();
        let keys = keys.clone();
        let q = q.clone();
        async move {
            let request = match provider {
                "unsplash" => client.get(format!("https://api.unsplash.com/search/photos?query={q}&per_page=12&content_filter=high{}", if landscape { "&orientation=landscape" } else { "" }))
                    .header("Authorization", format!("Client-ID {}", keys.unsplash.clone()?)).header("Accept-Version", "v1"),
                "pexels" => client.get(format!("https://api.pexels.com/v1/search?query={q}&per_page=12{}", if landscape { "&orientation=landscape" } else { "" }))
                    .header("Authorization", keys.pexels.clone()?),
                "pixabay" => client.get(format!("https://pixabay.com/api/?key={}&q={q}&image_type=photo&safesearch=true&per_page=12", encode(&keys.pixabay.clone()?))),
                _ => client.get(format!("https://wallhaven.cc/api/v1/search?q={q}&purity=100&categories=111&sorting=relevance{}", if landscape { "&atleast=1920x1080&ratios=landscape" } else { "" })),
            };
            let result = async {
                let data: Value = request.send().await.ok()?.error_for_status().ok()?.json().await.ok()?;
                Some(match provider { "unsplash" => parse_unsplash(&data), "pexels" => parse_pexels(&data), "pixabay" => parse_pixabay(&data), _ => parse_wallhaven(&data) })
            }.await;
            Some((provider, result))
        }
    };
    let results = futures_util::future::join_all(providers.iter().map(|p| run(p))).await;
    let mut lists = Vec::new();
    let mut failed = Vec::new();
    for (provider, result) in results.into_iter().flatten() {
        match result {
            // Wallpaper: só paisagem (as fontes nem sempre respeitam o filtro).
            Some(items) => lists.push(items.into_iter().filter(|i| !landscape || i.width >= i.height).collect::<Vec<_>>()),
            None => failed.push(provider.to_string()),
        }
    }
    let mut out = Vec::new();
    for i in 0..12 {
        for list in &lists { if let Some(item) = list.get(i) { out.push(item.clone()); } }
    }
    out.truncate(24);
    (out, failed)
}

/// Fontes com chave ainda não configurada que serviriam a este pedido.
pub fn missing(providers: &[&'static str], keys: &Keys) -> Vec<&'static str> {
    providers.iter().copied().filter(|p| match *p { "unsplash" => keys.unsplash.is_none(), "pexels" => keys.pexels.is_none(), "pixabay" => keys.pixabay.is_none(), _ => false }).collect()
}

/// Fontes sem API pública, abertas no navegador (id, nome).
pub const LINKS: &[(&str, &str)] = &[("pinterest", "Pinterest"), ("cosmos", "Cosmos"), ("same-energy", "same.energy"), ("arena", "Are.na")];

/// Links para abrir a busca no navegador (fontes sem API pública); com escolha, só os escolhidos.
pub fn links(query: &str, only: &[String]) -> Vec<(&'static str, String)> {
    let q = encode(query);
    let all = vec![
        ("Pinterest", format!("https://br.pinterest.com/search/pins/?q={q}")),
        ("Cosmos", format!("https://www.cosmos.so/search/elements/{q}")),
        ("same.energy", "https://same.energy/".to_string()),
        ("Are.na", "https://www.are.na/".to_string()),
    ];
    if only.is_empty() { return all; }
    all.into_iter().filter(|(name, _)| LINKS.iter().any(|(id, n)| n == name && only.iter().any(|o| o == id))).collect()
}

pub fn label(provider: &str) -> &'static str {
    match provider { "unsplash" => "Unsplash", "pexels" => "Pexels", "pixabay" => "Pixabay", _ => "Wallhaven" }
}

/// Resposta guardada na conversa (a grade aparece por cima, durante a sessão).
pub fn reply(kind: Kind, query: &str, items: &[Item], missing: &[&str], failed: &[String], links: &[(&str, String)], searched: bool, wanted: Option<usize>) -> String {
    let free = items.iter().filter(|i| i.license == "livre").count();
    if !searched && missing.is_empty() {
        let list: Vec<String> = links.iter().map(|(name, url)| format!("- [Abrir \"{query}\" no {name}]({url})")).collect();
        return format!("{} não têm busca pública por API; abra no navegador:\n\n{}", links.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", "), list.join("\n"));
    }
    let noun = |n: usize| if kind == Kind::Wallpaper { if n == 1 { "wallpaper" } else { "wallpapers" } } else if n == 1 { "imagem" } else { "imagens" };
    let mut out = if items.is_empty() {
        format!("Não encontrei {} para \"{query}\".", noun(2))
    } else {
        let short = wanted.filter(|w| *w > items.len()).map(|w| format!(" (você pediu {w}; foi o que achei)")).unwrap_or_default();
        format!("Encontrei {} {} de \"{query}\"{short}.", items.len(), noun(items.len()))
    };
    if !items.is_empty() {
        if free > 0 { out.push_str(&format!(" {free} {} de licença livre (Unsplash, Pexels, Pixabay): dá para usar em apresentações, mockups e peças, inclusive comerciais, sem pedir permissão; creditar o autor é uma gentileza. Não vale revender a foto sem alteração nem usá-la como marca.", if free == 1 { "é" } else { "são" })); }
        if items.len() > free { out.push_str(" As do Wallhaven têm direitos dos autores: use como referência ou papel de parede pessoal, não em trabalhos publicados."); }
        if kind == Kind::Free && free == 0 { out.push_str(" Nenhuma delas é de licença livre."); }
        out.push_str("\n\n");
        for item in items.iter().take(wanted.unwrap_or(8).min(12)) {
            let by = if item.author.is_empty() { String::new() } else { format!(" de {}", item.author) };
            out.push_str(&format!("- [{}{by}]({}) · {}\n", label(item.provider), item.page, if item.license == "livre" { "livre para usar" } else { "só referência" }));
        }
    }
    if !missing.is_empty() {
        let names: Vec<&str> = missing.iter().map(|p| label(p)).collect();
        out.push_str(&format!("\nPara buscar também em {}, cole a chave gratuita de cada um em Configurações › Busca de imagens.", names.join(", ")));
    }
    if !failed.is_empty() {
        let names: Vec<&str> = failed.iter().map(|p| label(p)).collect();
        out.push_str(&format!("\n{} não respondeu agora.", names.join(", ")));
    }
    out
}

/// Endereços de imagem aceitos ao salvar (as fontes da busca), para nunca baixar de outro lugar.
pub fn allowed_download(provider: &str, url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split('/').next().unwrap_or("");
    match provider {
        "unsplash" => host == "images.unsplash.com",
        "pexels" => host == "images.pexels.com",
        "pixabay" => host == "pixabay.com" || host == "cdn.pixabay.com",
        "wallhaven" => host == "w.wallhaven.cc",
        _ => false,
    }
}
pub fn allowed_ping(url: &str) -> bool {
    url.starts_with("https://api.unsplash.com/photos/")
}

/// Extensão pelo conteúdo (JPEG, PNG ou WebP); outros tipos são recusados.
pub fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) { Some("jpg") }
    else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") { Some("png") }
    else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" { Some("webp") }
    else { None }
}

/// Nome do arquivo salvo: fonte, autor e id.
pub fn file_name(item_provider: &str, author: &str, id: &str, extension: &str) -> String {
    let author = super::bases::csv::name(author).to_lowercase().replace('_', "-");
    let id: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(40).collect();
    if author.is_empty() || author.starts_with("t-") { format!("{item_provider}-{id}.{extension}") } else { format!("{item_provider}-{author}-{id}.{extension}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconhece_pedidos_e_extrai_termos() {
        assert_eq!(request("busca imagens de montanhas nevadas para uma apresentação"), Some(Kind::Free));
        assert_eq!(request("me acha um wallpaper de cidade cyberpunk"), Some(Kind::Wallpaper));
        assert_eq!(request("quero referências visuais de arquitetura brutalista"), Some(Kind::Reference));
        assert_eq!(request("fotos de cachorro dormindo"), Some(Kind::Reference), "começa pelo tipo");
        assert_eq!(request("cria uma imagem de um gato astronauta"), None, "geração, não busca");
        assert_eq!(request("qual o tamanho ideal de uma imagem no instagram?"), None);
        assert_eq!(terms("busca imagens de montanhas nevadas para uma apresentação"), "montanhas nevadas");
        assert_eq!(terms("me acha um wallpaper de cidade cyberpunk"), "cidade cyberpunk");
        assert_eq!(terms("fotos livres de café para mockup"), "cafe");
        assert_eq!(parse_terms(r#"{"pt":"montanhas","en":"snowy mountains"}"#), Some(("montanhas".into(), "snowy mountains".into())));
        assert_eq!(parse_terms(r#"{"pt":"wallpapers legais de one piece","en":"one piece wallpapers 4K"}"#), Some(("one piece".into(), "one piece".into())));
        assert_eq!(clean_terms("wallpapers"), "wallpapers", "não esvazia");
        assert_eq!(request("procura pra mim uns 5 wallpapers legais de one piece"), Some(Kind::Wallpaper));
        assert_eq!(request("me retorna 5 wallpapers de one piece"), Some(Kind::Wallpaper));
        assert_eq!(terms("me retorna 5 wallpapers de one piece"), "one piece");
        assert_eq!(request("me manda umas fotos de praia"), Some(Kind::Reference));
        assert_eq!(request("3 imagens de gato preto"), Some(Kind::Reference), "quantidade também indica pedido");
        assert_eq!(request("como troco o wallpaper do hyprland?"), None);
        assert_eq!(terms("procura pra mim uns 5 wallpapers legais de one piece"), "one piece");
        assert_eq!(count("procura pra mim uns 5 wallpapers legais de one piece"), Some(5));
        assert_eq!(count("me mostra três fotos de praia"), Some(3));
        assert_eq!(count("fotos de praia em 2024"), None);
        assert_eq!(count("100 imagens de gato"), Some(24));
    }

    #[test]
    fn le_as_respostas_das_fontes_com_licenca() {
        let unsplash = json!({"results":[{"id":"a1","width":4000,"height":3000,"urls":{"small":"https://images.unsplash.com/s","full":"https://images.unsplash.com/f"},
            "links":{"html":"https://unsplash.com/photos/a1","download_location":"https://api.unsplash.com/photos/a1/download"},"user":{"name":"Ana","links":{"html":"https://unsplash.com/@ana"}}}]});
        let item = &parse_unsplash(&unsplash)[0];
        assert_eq!((item.license, item.author.as_str()), ("livre", "Ana"));
        assert_eq!(item.page, "https://unsplash.com/photos/a1?utm_source=ayla&utm_medium=referral");
        assert!(allowed_ping(item.download_ping.as_deref().unwrap()) && allowed_download("unsplash", &item.full));
        let wallhaven = json!({"data":[{"id":"w1","purity":"sfw","resolution":"3840x2160","thumbs":{"large":"t"},"path":"https://w.wallhaven.cc/full/w1.jpg","url":"https://wallhaven.cc/w/w1","source":""},{"id":"x","purity":"nsfw"}]});
        let walls = parse_wallhaven(&wallhaven);
        assert_eq!((walls.len(), walls[0].license, walls[0].width), (1, "referencia", 3840), "só SFW");
        assert_eq!(parse_pexels(&json!({"photos":[{"id":7,"src":{"medium":"m","original":"o"},"url":"u","photographer":"Bia","photographer_url":"p","width":1,"height":2}]}))[0].license_name, "Licença Pexels");
        assert_eq!(parse_pixabay(&json!({"hits":[{"id":9,"webformatURL":"w","largeImageURL":"l","pageURL":"p","user":"caio","user_id":3}]}))[0].author_url, "https://pixabay.com/users/caio-3/");
        assert!(!allowed_download("unsplash", "https://evil.example/images.unsplash.com/x"));
        assert!(!allowed_download("pexels", "http://images.pexels.com/x"));
        assert!(!allowed_ping("https://api.unsplash.com/me"));
        let text = reply(Kind::Reference, "montanhas", &[item.clone(), walls[0].clone()], &["pexels"], &[], &[], true, None);
        assert!(reply(Kind::Wallpaper, "one piece", &[walls[0].clone()], &[], &[], &[], true, Some(5)).starts_with("Encontrei 1 wallpaper de \"one piece\" (você pediu 5"));
        assert!(reply(Kind::Reference, "gato", &[], &[], &[], &links("gato", &["pinterest".into()]), false, None).contains("[Abrir \"gato\" no Pinterest](https://br.pinterest.com/search/pins/?q=gato)"));
        assert!(text.contains("1 é de licença livre") && text.contains("Wallhaven têm direitos") && text.contains("Configurações › Busca de imagens"));
        assert_eq!(missing(&chosen(Kind::Free, &[]), &Keys { unsplash: Some("k".into()), ..Default::default() }), vec!["pexels", "pixabay"]);
        assert_eq!(chosen(Kind::Free, &["wallhaven".into(), "pinterest".into()]), vec!["wallhaven"], "escolha na barra vale mais que o tipo");
        assert_eq!(links("x", &["cosmos".into()]).len(), 1);
        assert_eq!(image_extension(b"\x89PNG\r\n\x1a\nxxxx"), Some("png"));
        assert_eq!(image_extension(b"<html>"), None);
        assert_eq!(file_name("unsplash", "Ana Lúcia", "a1", "jpg"), "unsplash-ana-lucia-a1.jpg");
        assert!(links("céu azul", &[])[0].1.ends_with("c%C3%A9u%20azul"));
    }

    /// Real (rede, sem chave): `cargo test --lib -- --ignored imagens_real --nocapture`
    #[test]
    #[ignore]
    fn imagens_real_wallhaven() {
        let client = reqwest::Client::builder().user_agent("Ayla/0.1 (assistente local)").build().unwrap();
        let (items, failed) = tauri::async_runtime::block_on(search(&client, Kind::Wallpaper, "mountain lake", &Keys::default(), &chosen(Kind::Wallpaper, &[])));
        for i in items.iter().take(3) { eprintln!("{} {}x{} {} {}", i.provider, i.width, i.height, i.license, i.page); }
        assert!(!items.is_empty() && failed.is_empty(), "{failed:?}");
        assert!(items.iter().all(|i| allowed_download(i.provider, &i.full) && i.width >= 1920));
    }
}
