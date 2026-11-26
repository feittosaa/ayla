//! Fichas de RPG em PDF (Athena + Iris). As fichas feitas no Canva não têm campos de formulário:
//! a Ayla usa um **mapa** (onde fica cada campo na página, montado a partir dos rótulos do PDF e
//! detectado sozinho e ajustável na Iris) e desenha uma **camada** por cima do PDF original: cobre o valor antigo com a cor do fundo e escreve o novo (texto, número ou
//! bolinhas). Tudo determinístico; a IA só traduz o pedido em mudanças de campo.
//!
//! Posições em pontos do PDF, com a origem no canto de cima à esquerda (como o `pdftotext -bbox`).
//! Leitura do texto e da imagem pelo poppler do sistema (`pdftotext` e `pdftoppm`, já usados nas
//! miniaturas); gravação com o `lopdf`.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Tipo do campo da ficha.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "tipo", rename_all = "lowercase")]
pub enum Tipo {
    /// Uma linha de texto (Nome, Ferramenta).
    Linha,
    /// Um número (Golpear 3, Vigor 20).
    Numero,
    /// Caixas com bolinhas preenchidas da esquerda para a direita (Estilos).
    Bolinhas { maximo: u32 },
    /// Texto livre em várias linhas (Condições, Técnicas).
    Bloco,
    /// Lista de traços: título e descrição.
    Tracos,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Campo {
    /// Chave na nota (sem acentos): "golpear", "vigor-atual".
    pub id: String,
    /// Como aparece para a pessoa e nas seções da nota: "Golpear".
    pub rotulo: String,
    /// Página dentro da ficha (0 = a primeira página da ficha).
    pub pagina: u32,
    /// [x0, y0, x1, y1] em pontos, origem em cima à esquerda.
    pub caixa: [f64; 4],
    #[serde(flatten)]
    pub tipo: Tipo,
}

/// Mapa de um modelo de ficha (vale para todas as fichas feitas com o mesmo modelo).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Mapa {
    pub nome: String,
    /// Páginas que uma ficha ocupa.
    pub paginas: u32,
    pub campos: Vec<Campo>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Traco {
    pub titulo: String,
    #[serde(default)]
    pub descricao: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Valor {
    Numero(i64),
    Texto(String),
    Tracos(Vec<Traco>),
}

pub type Valores = BTreeMap<String, Valor>;

/// Palavras que levam um pedido às fichas e o mantêm neste computador (as fichas ficam no cofre).
pub const PALAVRAS: &[&str] = &["ficha"];

pub fn pedido(message: &str) -> bool {
    let text = message.to_lowercase();
    PALAVRAS.iter().any(|w| text.contains(w))
}

// ---------------------------------------------------------------------------------------------
// Poppler: texto com posição e imagem da página.

/// Tempo máximo de cada programa do poppler (PDF quebrado não trava a Ayla).
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

fn run(mut command: Command) -> Result<(), String> {
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
        .map_err(|_| "A leitura de fichas precisa do poppler-utils (pdftotext e pdftoppm) instalado".to_string())?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => return Err("Não foi possível ler o PDF".into()),
            Ok(None) if start.elapsed() < TIMEOUT => std::thread::sleep(std::time::Duration::from_millis(15)),
            _ => { let _ = child.kill(); let _ = child.wait(); return Err("O PDF demorou demais para abrir".into()); }
        }
    }
}

/// Pasta temporária própria, apagada ao sair.
struct Temp(PathBuf);
impl Temp {
    fn new() -> Result<Self, String> {
        let dir = std::env::temp_dir().join(format!("ayla-ficha-{}", super::store::id()));
        fs::create_dir_all(&dir).map_err(|_| "Não foi possível preparar a leitura do PDF")?;
        Ok(Self(dir))
    }
}
impl Drop for Temp {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Palavra {
    pub texto: String,
    pub caixa: [f64; 4],
}

#[derive(Clone, Debug)]
pub struct Pagina {
    pub largura: f64,
    #[allow(dead_code)]
    pub altura: f64,
    pub palavras: Vec<Palavra>,
}

fn attr(tag: &str, name: &str) -> Option<f64> {
    let start = tag.find(&format!("{name}=\""))? + name.len() + 2;
    tag[start..].split('"').next()?.parse().ok()
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&apos;", "'").replace("&amp;", "&")
}

/// Lê a saída do `pdftotext -bbox` (uma página).
pub fn parse_bbox(html: &str) -> Option<Pagina> {
    let page = html.find("<page ")?;
    let tag = &html[page..page + html[page..].find('>')?];
    let (largura, altura) = (attr(tag, "width")?, attr(tag, "height")?);
    let mut palavras = Vec::new();
    for part in html[page..].split("<word ").skip(1) {
        let Some(end) = part.find('>') else { continue };
        let tag = &part[..end];
        let Some(text_end) = part.find("</word>") else { continue };
        let texto = unescape(&part[end + 1..text_end]);
        if let (Some(x0), Some(y0), Some(x1), Some(y1)) = (attr(tag, "xMin"), attr(tag, "yMin"), attr(tag, "xMax"), attr(tag, "yMax")) {
            if !texto.trim().is_empty() { palavras.push(Palavra { texto, caixa: [x0, y0, x1, y1] }); }
        }
    }
    Some(Pagina { largura, altura, palavras })
}

/// Palavras de uma página (1 = a primeira do PDF), com posição.
pub fn palavras(pdf: &Path, pagina: u32) -> Result<Pagina, String> {
    let temp = Temp::new()?;
    let out = temp.0.join("texto.html");
    let mut command = Command::new("pdftotext");
    command.args(["-bbox", "-enc", "UTF-8", "-f", &pagina.to_string(), "-l", &pagina.to_string()]).arg(pdf).arg(&out);
    run(command)?;
    parse_bbox(&fs::read_to_string(&out).map_err(|_| "Não foi possível ler o texto do PDF")?).ok_or_else(|| "Página não encontrada no PDF".into())
}

/// Palavras das páginas 1..=`ate` de uma vez (um só `pdftotext`); PDFs menores devolvem menos.
pub fn todas(pdf: &Path, ate: u32) -> Result<Vec<Pagina>, String> {
    let temp = Temp::new()?;
    let out = temp.0.join("texto.html");
    let mut command = Command::new("pdftotext");
    command.args(["-bbox", "-enc", "UTF-8", "-f", "1", "-l", &ate.max(1).to_string()]).arg(pdf).arg(&out);
    run(command)?;
    let html = fs::read_to_string(&out).map_err(|_| "Não foi possível ler o texto do PDF")?;
    let starts: Vec<usize> = html.match_indices("<page ").map(|(i, _)| i).collect();
    Ok(starts.iter().enumerate().filter_map(|(k, &i)| parse_bbox(&html[i..starts.get(k + 1).copied().unwrap_or(html.len())])).collect())
}

/// Imagem RGB da página, `escala` pixels por ponto.
pub struct Imagem {
    pub largura: usize,
    pub altura: usize,
    pub escala: f64,
    pixels: Vec<u8>,
}

impl Imagem {
    /// Lê um PPM binário (P6) como o do `pdftoppm`.
    pub fn from_ppm(data: &[u8], escala: f64) -> Option<Self> {
        let mut fields = Vec::new();
        let mut i = 0;
        while fields.len() < 4 && i < data.len() {
            while i < data.len() && data[i].is_ascii_whitespace() { i += 1; }
            if data.get(i) == Some(&b'#') { while i < data.len() && data[i] != b'\n' { i += 1; } continue; }
            let start = i;
            while i < data.len() && !data[i].is_ascii_whitespace() { i += 1; }
            fields.push(std::str::from_utf8(&data[start..i]).ok()?.to_string());
        }
        if fields.first()? != "P6" || fields.get(3)? != "255" { return None; }
        let (largura, altura): (usize, usize) = (fields[1].parse().ok()?, fields[2].parse().ok()?);
        let pixels = data.get(i + 1..i + 1 + largura * altura * 3)?.to_vec();
        Some(Self { largura, altura, escala, pixels })
    }
    #[cfg(test)]
    pub fn from_rgb(largura: usize, altura: usize, escala: f64, pixels: Vec<u8>) -> Self {
        Self { largura, altura, escala, pixels }
    }
    fn rgb(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.largura + x) * 3;
        [self.pixels[i], self.pixels[i + 1], self.pixels[i + 2]]
    }
    /// Pixel no ponto (x, y) da página, se estiver dentro da imagem.
    fn at(&self, x: f64, y: f64) -> Option<[u8; 3]> {
        let (px, py) = ((x * self.escala).floor(), (y * self.escala).floor());
        (px >= 0.0 && py >= 0.0 && (px as usize) < self.largura && (py as usize) < self.altura).then(|| self.rgb(px as usize, py as usize))
    }
    fn claro(&self, x: usize, y: usize) -> bool {
        self.rgb(x, y).iter().all(|&c| c > 200)
    }
    /// Célula clara que contém o ponto: a área de pixels claros ligados a ele, em pontos. `None`
    /// se o ponto for escuro ou se a área vazar para boa parte da página (não é uma célula).
    pub fn celula(&self, x: f64, y: f64) -> Option<[f64; 4]> {
        let (sx, sy) = ((x * self.escala) as isize, (y * self.escala) as isize);
        if sx < 0 || sy < 0 || sx as usize >= self.largura || sy as usize >= self.altura || !self.claro(sx as usize, sy as usize) {
            return None;
        }
        let limit = self.largura * self.altura * 3 / 5;
        let mut seen = vec![false; self.largura * self.altura];
        let mut stack = vec![(sx as usize, sy as usize)];
        seen[sy as usize * self.largura + sx as usize] = true;
        let (mut x0, mut y0, mut x1, mut y1) = (sx as usize, sy as usize, sx as usize, sy as usize);
        let mut count = 0;
        while let Some((cx, cy)) = stack.pop() {
            count += 1;
            if count > limit { return None; }
            x0 = x0.min(cx); x1 = x1.max(cx); y0 = y0.min(cy); y1 = y1.max(cy);
            let neighbors = [(cx.wrapping_sub(1), cy), (cx + 1, cy), (cx, cy.wrapping_sub(1)), (cx, cy + 1)];
            for (nx, ny) in neighbors {
                if nx < self.largura && ny < self.altura && !seen[ny * self.largura + nx] && self.claro(nx, ny) {
                    seen[ny * self.largura + nx] = true;
                    stack.push((nx, ny));
                }
            }
        }
        // Encostou na borda da página: é o fundo, não uma célula.
        if x0 == 0 || y0 == 0 || x1 + 1 >= self.largura || y1 + 1 >= self.altura { return None; }
        let s = self.escala;
        Some([x0 as f64 / s, y0 as f64 / s, (x1 + 1) as f64 / s, (y1 + 1) as f64 / s])
    }
    /// Fração de pixels escuros no retângulo.
    fn escuro(&self, caixa: [f64; 4]) -> f64 {
        let s = self.escala;
        let (x0, y0) = ((caixa[0] * s).max(0.0) as usize, (caixa[1] * s).max(0.0) as usize);
        let (x1, y1) = (((caixa[2] * s) as usize).min(self.largura), ((caixa[3] * s) as usize).min(self.altura));
        let (mut dark, mut total) = (0usize, 0usize);
        for y in y0..y1 {
            for x in x0..x1 {
                total += 1;
                let [r, g, b] = self.rgb(x, y);
                if (r as u32 + g as u32 + b as u32) / 3 < 110 { dark += 1; }
            }
        }
        if total == 0 { 0.0 } else { dark as f64 / total as f64 }
    }
    /// Cor mais comum entre os pixels claros do retângulo (o fundo de uma caixa, sem a bolinha).
    fn cor_da_caixa(&self, caixa: [f64; 4]) -> [u8; 3] {
        let s = self.escala;
        let (x0, y0) = ((caixa[0] * s).max(0.0) as usize, (caixa[1] * s).max(0.0) as usize);
        let (x1, y1) = (((caixa[2] * s) as usize).min(self.largura), ((caixa[3] * s) as usize).min(self.altura));
        let mut counts: BTreeMap<[u8; 3], usize> = BTreeMap::new();
        for y in y0..y1 { for x in x0..x1 {
            let c = self.rgb(x, y);
            if c.iter().all(|&v| v > 200) { *counts.entry(c).or_default() += 1; }
        } }
        counts.into_iter().max_by_key(|(_, n)| *n).map(|(c, _)| c).unwrap_or([255, 255, 255])
    }
    /// Cor do fundo em volta do retângulo: a mediana dos pixels claros de uma moldura logo fora dele.
    fn fundo(&self, caixa: [f64; 4]) -> [u8; 3] {
        let mut samples = Vec::new();
        let margin = 2.0;
        let step = 1.0 / self.escala.max(1.0);
        let (x0, y0, x1, y1) = (caixa[0] - margin, caixa[1] - margin, caixa[2] + margin, caixa[3] + margin);
        let mut t = x0;
        while t <= x1 {
            samples.extend(self.at(t, y0)); samples.extend(self.at(t, y1));
            t += step;
        }
        let mut t = y0;
        while t <= y1 {
            samples.extend(self.at(x0, t)); samples.extend(self.at(x1, t));
            t += step;
        }
        let mut light: Vec<[u8; 3]> = samples.iter().copied().filter(|c| c.iter().all(|&v| v > 170)).collect();
        if light.is_empty() { light = samples; }
        if light.is_empty() { return [255, 255, 255]; }
        light.sort_by_key(|c| c[0] as u32 + c[1] as u32 + c[2] as u32);
        light[light.len() / 2]
    }
}

/// Página em imagem (1 = a primeira do PDF), 2 pixels por ponto.
pub fn imagem(pdf: &Path, pagina: u32) -> Result<Imagem, String> {
    let temp = Temp::new()?;
    let stem = temp.0.join("pagina");
    let mut command = Command::new("pdftoppm");
    command.args(["-f", &pagina.to_string(), "-l", &pagina.to_string(), "-singlefile", "-r", "144"]).arg(pdf).arg(&stem);
    run(command)?;
    let data = fs::read(stem.with_extension("ppm")).map_err(|_| "Não foi possível desenhar a página")?;
    Imagem::from_ppm(&data, 2.0).ok_or_else(|| "Imagem da página inválida".into())
}

/// Página em PNG (para o editor do mapa e a prévia), `dpi` pontos por polegada.
pub fn png(pdf: &Path, pagina: u32, dpi: u32) -> Result<Vec<u8>, String> {
    let temp = Temp::new()?;
    let stem = temp.0.join("pagina");
    let mut command = Command::new("pdftoppm");
    command.args(["-f", &pagina.to_string(), "-l", &pagina.to_string(), "-singlefile", "-png", "-r", &dpi.to_string()]).arg(pdf).arg(&stem);
    run(command)?;
    fs::read(stem.with_extension("png")).map_err(|_| "Não foi possível desenhar a página".into())
}


/// Confere um mapa vindo da interface: nome, páginas, campos com chave única e caixas válidas.
pub fn validar(mapa: &Mapa) -> Result<(), String> {
    if mapa.nome.trim().is_empty() || mapa.nome.chars().count() > 80 { return Err("Dê um nome ao modelo de ficha (até 80 letras)".into()); }
    if !(1..=20).contains(&mapa.paginas) { return Err("Uma ficha ocupa de 1 a 20 páginas".into()); }
    if mapa.campos.is_empty() || mapa.campos.len() > 300 { return Err("O mapa precisa de 1 a 300 campos".into()); }
    let mut ids = std::collections::BTreeSet::new();
    for campo in &mapa.campos {
        if campo.id.is_empty() || slug(&campo.id) != campo.id || !ids.insert(campo.id.as_str()) {
            return Err(format!("Campo “{}”: a chave precisa ser única, sem acentos nem espaços", campo.rotulo));
        }
        let [x0, y0, x1, y1] = campo.caixa;
        if campo.rotulo.trim().is_empty() || campo.pagina >= mapa.paginas || !(x0 < x1 && y0 < y1) || campo.caixa.iter().any(|v| !v.is_finite() || *v < 0.0 || *v > 20_000.0) {
            return Err(format!("Campo “{}”: posição ou página inválida", campo.rotulo));
        }
        if let Tipo::Bolinhas { maximo } = campo.tipo { if !(1..=20).contains(&maximo) { return Err(format!("Campo “{}”: de 1 a 20 bolinhas", campo.rotulo)); } }
    }
    Ok(())
}

/// Valores vindos da interface ou da IA, conferidos contra o mapa: só campos que existem, do tipo
/// certo, bolinhas entre 0 e o máximo, textos com tamanho razoável.
pub fn conferir(mapa: &Mapa, valores: &Valores) -> Result<Valores, String> {
    let mut out = Valores::new();
    for (id, valor) in valores {
        let campo = mapa.campos.iter().find(|c| &c.id == id).ok_or_else(|| format!("A ficha não tem o campo “{id}”"))?;
        let ok = match (&campo.tipo, valor) {
            (Tipo::Numero, Valor::Numero(n)) => n.abs() <= 1_000_000,
            (Tipo::Bolinhas { maximo }, Valor::Numero(n)) => *n >= 0 && *n <= *maximo as i64,
            (Tipo::Linha | Tipo::Bloco, Valor::Texto(t)) => t.chars().count() <= 4000,
            (Tipo::Tracos, Valor::Tracos(list)) => list.len() <= 60 && list.iter().all(|t| t.titulo.chars().count() <= 200 && t.descricao.chars().count() <= 2000),
            _ => false,
        };
        if !ok { return Err(format!("Valor inválido para “{}”", campo.rotulo)); }
        out.insert(id.clone(), valor.clone());
    }
    Ok(out)
}

/// Mudanças de uma ficha para mostrar: (rótulo, antes, depois).
pub fn diferencas(mapa: &Mapa, antes: &Valores, depois: &Valores) -> Vec<(String, String, String)> {
    let show = |v: Option<&Valor>| match v {
        None => "—".to_string(),
        Some(Valor::Numero(n)) => n.to_string(),
        Some(Valor::Texto(t)) => t.clone(),
        Some(Valor::Tracos(list)) => list.iter().map(|t| t.titulo.clone()).collect::<Vec<_>>().join(", "),
    };
    mapa.campos.iter().filter(|c| antes.get(&c.id) != depois.get(&c.id)).map(|c| {
        if let (Tipo::Tracos, Some(Valor::Tracos(old)), Some(Valor::Tracos(new))) = (&c.tipo, antes.get(&c.id), depois.get(&c.id)) {
            let added: Vec<&str> = new.iter().filter(|t| !old.contains(t)).map(|t| t.titulo.as_str()).collect();
            let gone: Vec<&str> = old.iter().filter(|t| !new.iter().any(|n| n.titulo == t.titulo)).map(|t| t.titulo.as_str()).collect();
            let mut parts = Vec::new();
            if !added.is_empty() { parts.push(format!("+ {}", added.join(", "))); }
            if !gone.is_empty() { parts.push(format!("− {}", gone.join(", "))); }
            return (c.rotulo.clone(), String::new(), parts.join("; "));
        }
        (c.rotulo.clone(), show(antes.get(&c.id)), show(depois.get(&c.id)))
    }).collect()
}

/// "BARTHOLOMEW ROBERTS" → "Bartholomew Roberts" (nome da nota).
pub fn nome_proprio(text: &str) -> String {
    text.split_whitespace().map(titulo).collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------------------------
// Proposta do mapa a partir dos rótulos (em MAIÚSCULAS, como nas fichas do Canva).

fn centro(caixa: [f64; 4]) -> (f64, f64) {
    ((caixa[0] + caixa[2]) / 2.0, (caixa[1] + caixa[3]) / 2.0)
}
fn altura(caixa: [f64; 4]) -> f64 { caixa[3] - caixa[1] }
fn largura(caixa: [f64; 4]) -> f64 { caixa[2] - caixa[0] }
fn dentro(caixa: [f64; 4], ponto: (f64, f64)) -> bool {
    ponto.0 >= caixa[0] && ponto.0 <= caixa[2] && ponto.1 >= caixa[1] && ponto.1 <= caixa[3]
}

/// Rótulos: palavras em maiúsculas juntadas por linha ("MAS LUTA PRA SER"), sem os dois-pontos.
pub fn rotulos(palavras: &[Palavra]) -> Vec<Palavra> {
    let upper = |t: &str| t.chars().filter(|c| c.is_alphabetic()).count() >= 2 && t.chars().filter(|c| c.is_alphabetic()).all(char::is_uppercase);
    let mut words: Vec<&Palavra> = palavras.iter().filter(|p| upper(&p.texto)).collect();
    words.sort_by(|a, b| (centro(a.caixa).1 / 3.0).floor().partial_cmp(&(centro(b.caixa).1 / 3.0).floor()).unwrap_or(std::cmp::Ordering::Equal)
        .then(a.caixa[0].partial_cmp(&b.caixa[0]).unwrap_or(std::cmp::Ordering::Equal)));
    let mut out: Vec<Palavra> = Vec::new();
    for word in words {
        if let Some(last) = out.last_mut() {
            let same_line = (centro(last.caixa).1 - centro(word.caixa).1).abs() < altura(word.caixa) * 0.35;
            let gap = word.caixa[0] - last.caixa[2];
            if same_line && gap >= -1.0 && gap < altura(word.caixa) * 0.6 && !last.texto.ends_with(':') {
                last.texto = format!("{} {}", last.texto, word.texto);
                last.caixa = [last.caixa[0], last.caixa[1].min(word.caixa[1]), word.caixa[2], last.caixa[3].max(word.caixa[3])];
                continue;
            }
        }
        out.push(word.clone());
    }
    for label in &mut out {
        label.texto = label.texto.trim_end_matches(':').trim().to_string();
    }
    out
}

/// "VIGOR · ATUAL" → "vigor-atual" (sem acentos).
pub fn slug(text: &str) -> String {
    let plain: String = text.chars().map(|c| match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' | 'Á' | 'À' | 'Â' | 'Ã' | 'Ä' => 'a',
        'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' | 'Í' | 'Ì' | 'Î' | 'Ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' | 'Ó' | 'Ò' | 'Ô' | 'Õ' | 'Ö' => 'o',
        'ú' | 'ù' | 'û' | 'ü' | 'Ú' | 'Ù' | 'Û' | 'Ü' => 'u',
        'ç' | 'Ç' => 'c',
        other => other.to_ascii_lowercase(),
    }).collect();
    plain.split(|c: char| !c.is_ascii_alphanumeric()).filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-")
}

/// "PODEROSO" → "Poderoso"; "MAS LUTA PRA SER" → "Mas luta pra ser".
pub fn titulo(text: &str) -> String {
    let lower = text.to_lowercase();
    let mut chars = lower.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

/// Propõe os campos de uma página (em branco, de preferência) a partir dos rótulos e das células
/// claras da imagem: à direita do rótulo (caixas de bolinhas, número ao lado do ícone), abaixo
/// dele (Nome, Condições, Traços) ou no resto da própria célula (Alcance: …). Rótulos de seção
/// (cuja área de baixo contém outros rótulos) dão nome aos campos de dentro: "Vigor · Atual".
pub fn propor(pagina: &Pagina, imagem: &Imagem, indice: u32) -> Vec<Campo> {
    let labels = rotulos(&pagina.palavras);
    let has_label = |caixa: [f64; 4], skip: usize| labels.iter().enumerate().any(|(i, l)| i != skip && dentro(caixa, centro(l.caixa)));
    // Área logo abaixo de uma caixa (tenta algumas distâncias, para passar bordas grossas).
    let below = |caixa: [f64; 4]| [2.5, 5.0, 8.0, 12.0].iter().find_map(|d| imagem.celula(centro(caixa).0, caixa[3] + d).filter(|c| c[1] >= caixa[3] - 1.0));
    struct Proposta { label: usize, caixa: [f64; 4], tipo: Tipo }
    let mut found: Vec<Proposta> = Vec::new();
    let mut sections: Vec<(usize, [f64; 4])> = Vec::new();
    for (i, label) in labels.iter().enumerate() {
        let (_, cy) = centro(label.caixa);
        let h = altura(label.caixa);
        let own = imagem.celula(label.caixa[2] + 2.0, cy).or_else(|| imagem.celula(label.caixa[2] + 4.0, cy));
        // Células à direita, na mesma linha (para na próxima que tiver rótulo).
        let mut neighbors: Vec<[f64; 4]> = Vec::new();
        if let Some(start) = own.filter(|c| !has_label(*c, i)).or(own) {
            let mut x = start[2] + 0.5;
            let limit = pagina.largura;
            while x < limit && neighbors.len() < 12 {
                match imagem.celula(x, cy) {
                    Some(cell) if has_label(cell, i) => break,
                    Some(cell) if largura(cell) >= 6.0 && altura(cell) >= h * 0.6 && altura(cell) <= h * 4.0 && cell[0] >= start[2] - 1.0 => {
                        x = cell[2] + 0.5;
                        neighbors.push(cell);
                    }
                    Some(cell) => x = cell[2].max(x) + 0.5,
                    None => {
                        // Borda ou ícone: segue até 12 pontos; mais que isso é fim da seção.
                        let gap_start = x;
                        while x < limit && x - gap_start < 12.0 && imagem.celula(x, cy).is_none() { x += 0.5; }
                        if imagem.celula(x, cy).is_none() { break; }
                    }
                }
            }
        }
        // Faixa de título: a célula do rótulo é baixa, ou logo abaixo há outra da mesma largura e
        // sem rótulo (o valor vai embaixo, como em Nome | Pronomes | Ferramenta).
        let strip = own.is_some_and(|c| altura(c) < h * 1.65 || below(c).is_some_and(|b| {
            (largura(b) - largura(c)).abs() < largura(c) * 0.1 && b[1] - c[3] < 6.0 && altura(b) >= altura(c) * 0.8 && !has_label(b, i)
        }));
        let similar = |a: &[f64; 4], b: &[f64; 4]| (largura(*a) - largura(*b)).abs() < largura(*a).max(largura(*b)) * 0.25 && (altura(*a) - altura(*b)).abs() < 4.0;
        if neighbors.len() >= 2 && neighbors.windows(2).all(|w| similar(&w[0], &w[1])) && largura(neighbors[0]) < altura(neighbors[0]) * 2.5 {
            let first = neighbors[0];
            let last = *neighbors.last().unwrap_or(&first);
            found.push(Proposta { label: i, caixa: [first[0], first[1], last[2], last[3]], tipo: Tipo::Bolinhas { maximo: neighbors.len() as u32 } });
            continue;
        }
        if neighbors.len() == 1 && !strip {
            let cell = neighbors[0];
            let tipo = if largura(cell) < altura(cell) * 2.5 { Tipo::Numero } else { Tipo::Linha };
            found.push(Proposta { label: i, caixa: cell, tipo });
            continue;
        }
        let rest = own.filter(|c| !strip && c[2] - label.caixa[2] > 30.0 && !has_label([label.caixa[2], c[1], c[2], c[3]], i));
        if let Some(cell) = rest {
            let caixa = [label.caixa[2] + 3.0, cell[1], cell[2], cell[3]];
            let tipo = if altura(cell) > h * 2.5 { Tipo::Bloco } else { Tipo::Linha };
            found.push(Proposta { label: i, caixa, tipo });
            continue;
        }
        let base = own.filter(|c| c[1] <= label.caixa[1] + 1.0).unwrap_or(label.caixa);
        let Some(area) = below(base).or_else(|| below(label.caixa)) else { continue };
        let inside: Vec<usize> = labels.iter().enumerate().filter(|(j, l)| *j != i && dentro(area, centro(l.caixa))).map(|(j, _)| j).collect();
        if inside.is_empty() {
            let tipo = if altura(area) > h * 2.8 { Tipo::Bloco } else { Tipo::Linha };
            found.push(Proposta { label: i, caixa: area, tipo });
        } else if inside.iter().all(|&j| (centro(labels[j].caixa).1 - cy).abs() < h) {
            // Vários rótulos lado a lado sobre a mesma área (Vigor: Atual | Max): uma coluna cada.
            let mut row: Vec<usize> = inside.clone();
            row.push(i);
            row.sort_by(|a, b| labels[*a].caixa[0].partial_cmp(&labels[*b].caixa[0]).unwrap_or(std::cmp::Ordering::Equal));
            let pos = row.iter().position(|&j| j == i).unwrap_or(0);
            let left = if pos == 0 { area[0] } else { (labels[row[pos - 1]].caixa[2] + labels[i].caixa[0]) / 2.0 };
            let right = if pos + 1 == row.len() { area[2] } else { (labels[i].caixa[2] + labels[row[pos + 1]].caixa[0]) / 2.0 };
            found.push(Proposta { label: i, caixa: [left, area[1], right, area[3]], tipo: Tipo::Numero });
        } else {
            sections.push((i, area));
        }
    }
    // Nomes: o rótulo, com a seção na frente quando ele se repete ("Vigor · Atual").
    let repeated: Vec<String> = found.iter().map(|p| labels[p.label].texto.clone()).collect();
    let count = |text: &str| repeated.iter().filter(|t| *t == text).count();
    let mut used: Vec<String> = Vec::new();
    found.into_iter().map(|p| {
        let label = &labels[p.label];
        let section = sections.iter().filter(|(_, area)| dentro(*area, centro(label.caixa)))
            .min_by(|a, b| (largura(a.1) * altura(a.1)).partial_cmp(&(largura(b.1) * altura(b.1))).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(s, _)| titulo(&labels[*s].texto));
        let mut rotulo = titulo(&label.texto);
        if count(&label.texto) > 1 {
            rotulo = match section { Some(s) => format!("{s} · {rotulo}"), None => rotulo };
        }
        let mut id = slug(&rotulo);
        let mut n = 2;
        while used.contains(&id) { id = format!("{}-{n}", slug(&rotulo)); n += 1; }
        if used.iter().any(|u| slug(u) == slug(&rotulo)) && n > 2 { rotulo = format!("{rotulo} {}", n - 1); }
        used.push(id.clone());
        let round = |v: f64| (v * 10.0).round() / 10.0;
        Campo { id, rotulo, pagina: indice, caixa: p.caixa.map(round), tipo: p.tipo }
    }).collect()
}

// ---------------------------------------------------------------------------------------------
// Leitura dos valores de uma ficha preenchida.

/// Linhas de texto dentro da caixa: (texto, altura da letra, topo).
fn linhas(pagina: &Pagina, caixa: [f64; 4]) -> Vec<(String, f64, f64)> {
    let mut words: Vec<&Palavra> = pagina.palavras.iter().filter(|p| dentro(caixa, centro(p.caixa))).collect();
    words.sort_by(|a, b| a.caixa[1].partial_cmp(&b.caixa[1]).unwrap_or(std::cmp::Ordering::Equal));
    let mut lines: Vec<Vec<&Palavra>> = Vec::new();
    for word in words {
        match lines.last_mut() {
            Some(line) if (centro(line[0].caixa).1 - centro(word.caixa).1).abs() < altura(word.caixa) * 0.5 => line.push(word),
            _ => lines.push(vec![word]),
        }
    }
    lines.into_iter().map(|mut line| {
        line.sort_by(|a, b| a.caixa[0].partial_cmp(&b.caixa[0]).unwrap_or(std::cmp::Ordering::Equal));
        let h = line.iter().map(|w| altura(w.caixa)).fold(0.0, f64::max);
        let top = line.iter().map(|w| w.caixa[1]).fold(f64::MAX, f64::min);
        (line.iter().map(|w| w.texto.as_str()).collect::<Vec<_>>().join(" "), h, top)
    }).collect()
}

/// Valor de um campo na página (`None` quando está vazio).
pub fn ler_campo(campo: &Campo, pagina: &Pagina, imagem: Option<&Imagem>) -> Option<Valor> {
    let lines = linhas(pagina, campo.caixa);
    match &campo.tipo {
        Tipo::Linha => (!lines.is_empty()).then(|| Valor::Texto(lines.iter().map(|l| l.0.as_str()).collect::<Vec<_>>().join(" "))),
        Tipo::Numero => lines.iter().flat_map(|l| l.0.split_whitespace()).find_map(|w| w.trim_start_matches('+').parse::<i64>().ok()).map(Valor::Numero),
        Tipo::Bloco => (!lines.is_empty()).then(|| Valor::Texto(lines.iter().map(|l| l.0.as_str()).collect::<Vec<_>>().join("\n"))),
        Tipo::Bolinhas { maximo } => {
            let imagem = imagem?;
            let cells = colunas(campo.caixa, *maximo);
            Some(Valor::Numero(cells.iter().filter(|c| imagem.escuro(encolher(**c, 0.3)) > 0.3).count() as i64))
        }
        Tipo::Tracos => {
            if lines.is_empty() { return None; }
            let small = lines.iter().map(|l| l.1).fold(f64::MAX, f64::min);
            let mut tracos: Vec<Traco> = Vec::new();
            let mut last_top = f64::MIN;
            for (text, h, top) in &lines {
                if *h > small * 1.15 {
                    // Título que continua na linha de baixo (também grande): junta.
                    match tracos.last_mut() {
                        Some(t) if t.descricao.is_empty() && *top - last_top < h * 1.6 => t.titulo = format!("{} {text}", t.titulo),
                        _ => tracos.push(Traco { titulo: text.clone(), descricao: String::new() }),
                    }
                } else {
                    match tracos.last_mut() {
                        Some(t) if t.descricao.is_empty() => t.descricao = text.clone(),
                        Some(t) => {
                            let paragraph = *top - last_top > h * 2.2;
                            t.descricao = format!("{}{}{text}", t.descricao, if paragraph { "\n" } else { " " });
                        }
                        None => tracos.push(Traco { titulo: String::new(), descricao: text.clone() }),
                    }
                }
                last_top = *top;
            }
            Some(Valor::Tracos(tracos))
        }
    }
}

fn colunas(caixa: [f64; 4], n: u32) -> Vec<[f64; 4]> {
    let n = n.max(1);
    let w = largura(caixa) / n as f64;
    (0..n).map(|i| [caixa[0] + w * i as f64, caixa[1], caixa[0] + w * (i + 1) as f64, caixa[3]]).collect()
}

fn encolher(caixa: [f64; 4], fracao: f64) -> [f64; 4] {
    let (dx, dy) = (largura(caixa) * fracao, altura(caixa) * fracao);
    [caixa[0] + dx, caixa[1] + dy, caixa[2] - dx, caixa[3] - dy]
}

/// Valores de uma ficha (`primeira`: página do PDF onde ela começa, 1 = a primeira).
pub fn ler(mapa: &Mapa, pdf: &Path, primeira: u32) -> Result<Valores, String> {
    let mut out = Valores::new();
    for page in 0..mapa.paginas {
        let campos: Vec<&Campo> = mapa.campos.iter().filter(|c| c.pagina == page).collect();
        if campos.is_empty() { continue; }
        let pagina = palavras(pdf, primeira + page)?;
        let imagem = if campos.iter().any(|c| matches!(c.tipo, Tipo::Bolinhas { .. })) { Some(imagem(pdf, primeira + page)?) } else { None };
        for campo in campos {
            if let Some(valor) = ler_campo(campo, &pagina, imagem.as_ref()) { out.insert(campo.id.clone(), valor); }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Detecção: onde começa cada ficha de um PDF e de onde sai o mapa, sem preparo manual.

/// Rótulos da página (o que identifica o tipo de página do modelo).
pub fn assinatura(pagina: &Pagina) -> BTreeSet<String> {
    rotulos(&pagina.palavras).into_iter().map(|l| l.texto).collect()
}

/// Fração dos rótulos de `a` que aparecem em `b`.
pub fn contido(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    if a.is_empty() { 0.0 } else { a.intersection(b).count() as f64 / a.len() as f64 }
}

/// Mesmo tipo de página: a maior parte dos rótulos da menor está na outra (os valores de cada
/// personagem também viram "rótulos" quando estão em maiúsculas, por isso não se exige tudo).
fn parecidas(a: &BTreeSet<String>, b: &BTreeSet<String>) -> bool {
    let comuns = a.intersection(b).count();
    comuns >= 5 && comuns as f64 >= a.len().min(b.len()) as f64 * 0.6
}

/// Fichas de um PDF com várias do mesmo modelo: começam nas páginas do mesmo tipo da primeira.
/// Devolve (páginas por ficha, inícios; 1 = a primeira página). Uma ficha só: o PDF inteiro.
pub fn separar(assinaturas: &[BTreeSet<String>]) -> (u32, Vec<u32>) {
    let n = assinaturas.len();
    if n == 0 { return (1, Vec::new()); }
    let inicios: Vec<usize> = (0..n).filter(|&i| i == 0 || parecidas(&assinaturas[0], &assinaturas[i])).collect();
    if inicios.len() < 2 { return (n as u32, vec![1]); }
    let por = inicios.windows(2).map(|w| w[1] - w[0]).min().unwrap_or(1);
    (por as u32, inicios.into_iter().map(|i| i as u32 + 1).collect())
}

/// Quanto uma ficha em branco (`branco`, uma assinatura por página) combina com as páginas de
/// uma preenchida a partir de `inicio` (0 = a primeira): média dos rótulos dela encontrados.
pub fn combina(branco: &[BTreeSet<String>], paginas: &[BTreeSet<String>], inicio: usize) -> f64 {
    if branco.is_empty() || inicio + branco.len() > paginas.len() { return 0.0; }
    branco.iter().enumerate().map(|(j, b)| contido(b, &paginas[inicio + j])).sum::<f64>() / branco.len() as f64
}

/// Inícios das fichas de um PDF feitas com a ficha em branco `branco`.
pub fn inicios_do_modelo(branco: &[BTreeSet<String>], paginas: &[BTreeSet<String>]) -> Vec<u32> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < paginas.len() {
        if combina(branco, paginas, i) >= 0.8 { out.push(i as u32 + 1); i += branco.len(); } else { i += 1; }
    }
    out
}

/// Página "em branco" com as palavras que se repetem em todas as fichas (as do modelo); os
/// valores de cada personagem ficam de fora.
pub fn comum(paginas: &[&Pagina]) -> Option<Pagina> {
    let (first, rest) = paginas.split_first()?;
    let sets: Vec<BTreeSet<&str>> = rest.iter().map(|p| p.palavras.iter().map(|w| w.texto.as_str()).collect()).collect();
    let mut out = (*first).clone();
    out.palavras.retain(|w| sets.iter().all(|s| s.contains(w.texto.as_str())));
    Some(out)
}

/// Chaves únicas entre as páginas ("nome" da página 2 vira "nome-2").
pub fn chaves_unicas(campos: &mut [Campo]) {
    let mut used = std::collections::BTreeSet::new();
    for campo in campos {
        let base = campo.id.clone();
        let mut n = 2;
        while !used.insert(campo.id.clone()) { campo.id = format!("{base}-{n}"); n += 1; }
    }
}

/// Campo com o nome do personagem: "Nome" na primeira página, ou o primeiro texto curto dela.
pub fn campo_nome(mapa: &Mapa) -> Option<&Campo> {
    let curtos = || mapa.campos.iter().filter(|c| c.pagina == 0 && c.tipo == Tipo::Linha);
    curtos().find(|c| c.id == "nome").or_else(|| curtos().next())
}

/// Nome do personagem lido na primeira página da ficha (`None` se o campo estiver vazio).
pub fn nome_na_pagina(mapa: &Mapa, pagina: &Pagina) -> Option<String> {
    match ler_campo(campo_nome(mapa)?, pagina, None)? {
        Valor::Texto(t) if !t.trim().is_empty() => Some(nome_proprio(t.lines().next().unwrap_or(&t))),
        _ => None,
    }
}

/// Palavras do pedido que indicam um personagem: "bart" acha "Bartholomew Roberts" (começo de
/// palavra, com 4 letras ou mais; palavras inteiras a partir de 3). Devolve quantas batem.
pub fn nome_bate(nome: &str, pedido: &str) -> usize {
    let nome_words: Vec<String> = slug(nome).split('-').filter(|w| w.len() >= 3).map(str::to_string).collect();
    let mut usadas = BTreeSet::new();
    for w in slug(pedido).split('-') {
        if w.len() < 3 { continue; }
        if let Some(i) = nome_words.iter().position(|n| n == w || (w.len() >= 4 && n.starts_with(w))) { usadas.insert(i); }
    }
    usadas.len()
}

/// Larguras da Helvetica Bold (WinAnsi, 32 a 255), em milésimos do tamanho da letra; tiradas da
/// Nimbus Sans Bold (URW), de métrica igual.
const WIDTHS: [u16; 224] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278,
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611,
    975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778,
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584, 556,
    333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611,
    611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584, 0,
    556, 0, 278, 556, 500, 1000, 556, 556, 333, 1000, 667, 333, 1000, 0, 611, 0,
    0, 278, 278, 500, 500, 350, 556, 1000, 333, 1000, 556, 333, 944, 0, 500, 667,
    278, 333, 556, 556, 556, 556, 280, 556, 333, 737, 370, 556, 584, 333, 737, 333,
    400, 584, 333, 333, 333, 611, 556, 278, 333, 333, 365, 556, 834, 834, 834, 611,
    722, 722, 722, 722, 722, 722, 1000, 722, 667, 667, 667, 667, 278, 278, 278, 278,
    722, 722, 778, 778, 778, 778, 778, 584, 778, 722, 722, 722, 722, 667, 667, 611,
    556, 556, 556, 556, 556, 556, 889, 556, 556, 556, 556, 556, 278, 278, 278, 278,
    611, 611, 611, 611, 611, 611, 611, 584, 611, 611, 611, 611, 611, 556, 611, 556,
];

/// Texto em WinAnsi (o que não existe nela vira "?").
pub fn winansi(text: &str) -> Vec<u8> {
    text.chars().map(|c| {
        let code = c as u32;
        match code {
            32..=126 | 160..=255 => code as u8,
            _ => match c {
                '€' => 0x80, '‚' => 0x82, '„' => 0x84, '…' => 0x85, '‘' => 0x91, '’' => 0x92, '“' => 0x93, '”' => 0x94,
                '•' => 0x95, '–' => 0x96, '—' => 0x97, '™' => 0x99, 'Œ' => 0x8C, 'œ' => 0x9C,
                _ => b'?',
            },
        }
    }).collect()
}

/// Largura do texto em pontos.
pub fn medir(text: &str, size: f64) -> f64 {
    winansi(text).iter().map(|&b| if b >= 32 { WIDTHS[(b - 32) as usize] as f64 } else { 0.0 }).sum::<f64>() * size / 1000.0
}

fn pdf_string(text: &str) -> String {
    let mut out = String::from("(");
    for b in winansi(text) {
        match b {
            b'(' | b')' | b'\\' => { out.push('\\'); out.push(b as char); }
            32..=126 => out.push(b as char),
            other => out.push_str(&format!("\\{other:03o}")),
        }
    }
    out.push(')');
    out
}

/// Quebra o texto em linhas que cabem na largura (palavras longas demais ficam na linha sozinhas).
pub fn quebrar(text: &str, size: f64, max: f64) -> Vec<String> {
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if medir(&candidate, size) <= max || line.is_empty() { line = candidate; } else { out.push(std::mem::take(&mut line)); line = word.to_string(); }
        }
        out.push(line);
    }
    out
}

/// Comandos de desenho da camada de uma página (em coordenadas do PDF: origem embaixo).
struct Tela { ops: String, altura: f64, x0: f64 }
impl Tela {
    fn y(&self, top: f64) -> f64 { self.altura - top }
    fn retangulo(&mut self, caixa: [f64; 4], cor: [u8; 3]) {
        let [r, g, b] = cor.map(|c| c as f64 / 255.0);
        self.ops.push_str(&format!("{r:.3} {g:.3} {b:.3} rg {:.2} {:.2} {:.2} {:.2} re f\n", self.x0 + caixa[0], self.y(caixa[3]), largura(caixa), altura(caixa)));
    }
    fn bolinha(&mut self, cx: f64, cy: f64, raio: f64) {
        let (x, y, k) = (self.x0 + cx, self.y(cy), raio * 0.5523);
        self.ops.push_str(&format!(
            "0.08 0.08 0.1 rg {:.2} {:.2} m {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c f\n",
            x + raio, y,
            x + raio, y + k, x + k, y + raio, x, y + raio,
            x - k, y + raio, x - raio, y + k, x - raio, y,
            x - raio, y - k, x - k, y - raio, x, y - raio,
            x + k, y - raio, x + raio, y - k, x + raio, y));
    }
    fn texto(&mut self, text: &str, x: f64, baseline_top: f64, size: f64) {
        self.ops.push_str(&format!("BT 0.08 0.08 0.1 rg /AylaF {size:.2} Tf {:.2} {:.2} Td {} Tj ET\n", self.x0 + x, self.y(baseline_top), pdf_string(text)));
    }
}

/// Maior tamanho (até `max`) com que as linhas cabem na caixa.
fn caber(caixa: [f64; 4], max: f64, linhas_de: impl Fn(f64) -> f64) -> f64 {
    let mut size = max;
    while size > 4.0 && linhas_de(size) > altura(caixa) { size -= 0.25; }
    size
}

/// Como o texto original do campo estava na página: tamanho da letra (na Helvetica que a Ayla usa,
/// para a mesma altura) e de onde a onde ia na horizontal.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Dica { tamanho: f64, faixa: Option<(f64, f64)> }

/// Altura da caixa de uma palavra da Helvetica Bold no `pdftotext`, em tamanhos da letra.
const ALTURA_HELVETICA: f64 = 0.925;

/// Dica a partir das palavras do PDF original dentro do campo (`None` se ele estava vazio).
fn dica(campo: &Campo, original: &Pagina) -> Option<Dica> {
    let words: Vec<&Palavra> = original.palavras.iter().filter(|p| dentro(campo.caixa, centro(p.caixa))).collect();
    let mut alturas: Vec<f64> = words.iter().map(|p| altura(p.caixa)).collect();
    alturas.sort_by(f64::total_cmp);
    let mediana = *alturas.get(alturas.len() / 2)?;
    let x0 = words.iter().map(|p| p.caixa[0]).fold(f64::MAX, f64::min);
    let x1 = words.iter().map(|p| p.caixa[2]).fold(f64::MIN, f64::max);
    Some(Dica { tamanho: mediana / ALTURA_HELVETICA, faixa: Some((x0, x1)) })
}

/// Desenha um campo na tela. Com `dica`, segue o tamanho e o alinhamento do texto original.
fn desenhar(tela: &mut Tela, campo: &Campo, valor: Option<&Valor>, dica: Option<Dica>) {
    let caixa = campo.caixa;
    let pad = 2.0;
    let inner = [caixa[0] + pad, caixa[1] + pad * 0.5, caixa[2] - pad, caixa[3] - pad * 0.5];
    match (&campo.tipo, valor) {
        (Tipo::Bolinhas { maximo }, Some(Valor::Numero(n))) => {
            for cell in colunas(caixa, *maximo).iter().take((*n).clamp(0, *maximo as i64) as usize) {
                let (cx, cy) = centro(*cell);
                tela.bolinha(cx, cy, largura(*cell).min(altura(*cell)) * 0.38);
            }
        }
        (Tipo::Numero, Some(Valor::Numero(n))) => {
            let text = n.to_string();
            let padrao = (altura(inner) * 0.72).min(11.0);
            let size = dica.map_or(padrao, |d| d.tamanho.min(altura(inner))).max(5.0);
            let size = if medir(&text, size) > largura(inner) { size * largura(inner) / medir(&text, size) } else { size };
            // No mesmo centro do número antigo (alinhado com os vizinhos), dentro da caixa.
            let meio = dica.and_then(|d| d.faixa).map_or(centro(inner).0, |(a, b)| (a + b) / 2.0);
            let x = (meio - medir(&text, size) / 2.0).clamp(inner[0], (inner[2] - medir(&text, size)).max(inner[0]));
            tela.texto(&text, x, centro(inner).1 + size * 0.36, size);
        }
        (Tipo::Linha, Some(Valor::Texto(t))) => {
            let max = dica.map_or((altura(inner) * 0.7).min(11.0), |d| d.tamanho.min(altura(inner)));
            let x = dica.and_then(|d| d.faixa).map_or(inner[0], |(a, _)| a.clamp(inner[0], inner[2]));
            let room = inner[2] - x;
            let size = if medir(t, max) > room { (max * room / medir(t, max)).max(4.0) } else { max };
            tela.texto(t, x, centro(inner).1 + size * 0.36, size);
        }
        (Tipo::Bloco, Some(Valor::Texto(t))) => {
            let size = caber(inner, dica.map_or(9.0, |d| d.tamanho.min(9.0)), |s| quebrar(t, s, largura(inner)).len() as f64 * s * 1.2);
            for (i, line) in quebrar(t, size, largura(inner)).iter().enumerate() {
                tela.texto(line, inner[0], inner[1] + size * (1.2 * i as f64 + 0.95), size);
            }
        }
        (Tipo::Tracos, Some(Valor::Tracos(list))) => {
            // Título maior e centralizado; descrição menor, centralizada embaixo dele.
            let height = |s: f64| list.iter().map(|t| {
                let title = quebrar(&t.titulo, s * 1.25, largura(inner)).len() as f64 * s * 1.25 * 1.15;
                let desc = if t.descricao.is_empty() { 0.0 } else { quebrar(&t.descricao, s, largura(inner)).len() as f64 * s * 1.2 };
                title + desc + s * 1.1
            }).sum::<f64>();
            let size = caber(inner, 8.5, height);
            let mut top = inner[1];
            for t in list {
                let ts = size * 1.25;
                for line in quebrar(&t.titulo, ts, largura(inner)) {
                    tela.texto(&line, inner[0] + (largura(inner) - medir(&line, ts)) / 2.0, top + ts * 0.95, ts);
                    top += ts * 1.15;
                }
                if !t.descricao.is_empty() {
                    for line in quebrar(&t.descricao, size, largura(inner)) {
                        tela.texto(&line, inner[0] + (largura(inner) - medir(&line, size)) / 2.0, top + size * 0.95, size);
                        top += size * 1.2;
                    }
                }
                top += size * 1.1;
            }
        }
        _ => {}
    }
}

/// Uma ficha para desenhar: o mapa, os valores e a página do PDF onde ela começa (1 = a primeira).
pub struct Ficha<'a> {
    pub mapa: &'a Mapa,
    pub valores: &'a Valores,
    pub primeira: u32,
}

/// Camada de uma página: cobre o texto antigo dentro dos campos (e as caixas das bolinhas) com a
/// cor do fundo em volta e desenha os valores. `pagina` é o texto que sobrou depois de tirar o
/// antigo; `original`/`imagem` são do PDF original (o tamanho e a posição do texto antigo).
fn camada(campos: &[(&Campo, Option<&Valor>)], todos: &[&Campo], pagina: &Pagina, original: &Pagina, imagem: &Imagem, altura_pdf: f64, x0: f64) -> String {
    let mut tela = Tela { ops: String::new(), altura: altura_pdf, x0 };
    for (campo, _) in campos {
        match &campo.tipo {
            Tipo::Bolinhas { maximo } => for cell in colunas(campo.caixa, *maximo) {
                let cor = imagem.cor_da_caixa(encolher(cell, 0.1));
                let (dx, dy) = (largura(cell) * 0.07, altura(cell) * 0.03);
                tela.retangulo([cell[0] + dx, cell[1] + dy, cell[2] - dx, cell[3] - dy], cor);
            },
            _ => for word in pagina.palavras.iter().filter(|p| dentro(campo.caixa, centro(p.caixa))) {
                let c = word.caixa;
                let caixa = [c[0] - 0.8, c[1] - 0.6, c[2] + 0.8, c[3] + 0.6];
                let cor = imagem.fundo(caixa);
                tela.retangulo(caixa, cor);
            },
        }
    }
    // Campo vazio no original: o tamanho mediano dos campos do mesmo tipo nesta página.
    let do_tipo = |tipo: &Tipo| {
        let mut sizes: Vec<f64> = todos.iter()
            .filter(|c| std::mem::discriminant(&c.tipo) == std::mem::discriminant(tipo))
            .filter_map(|c| dica(c, original).map(|d| d.tamanho)).collect();
        sizes.sort_by(f64::total_cmp);
        sizes.get(sizes.len() / 2).copied()
    };
    for (campo, valor) in campos {
        let d = dica(campo, original).or_else(|| do_tipo(&campo.tipo).map(|tamanho| Dica { tamanho, faixa: None }));
        desenhar(&mut tela, campo, *valor, d);
    }
    tela.ops
}

fn lopdf_error(_: lopdf::Error) -> String { "Não foi possível montar o PDF da ficha".into() }

/// Recursos da página como dicionário próprio (copiando os herdados da árvore de páginas).
fn recursos(doc: &lopdf::Document, page: lopdf::ObjectId) -> Result<lopdf::Dictionary, String> {
    use lopdf::Object;
    let mut node = Some(page);
    while let Some(id) = node {
        let dict = doc.get_dictionary(id).map_err(lopdf_error)?;
        match dict.get(b"Resources") {
            Ok(Object::Dictionary(d)) => return Ok(d.clone()),
            Ok(Object::Reference(r)) => return doc.get_dictionary(*r).cloned().map_err(lopdf_error),
            _ => node = dict.get(b"Parent").and_then(Object::as_reference).ok(),
        }
    }
    Ok(lopdf::Dictionary::new())
}

/// MediaBox da página (procurando nos pais): [x0, y0, x1, y1].
fn media_box(doc: &lopdf::Document, page: lopdf::ObjectId) -> [f64; 4] {
    use lopdf::Object;
    let mut node = Some(page);
    while let Some(id) = node {
        let Ok(dict) = doc.get_dictionary(id) else { break };
        if let Ok(array) = dict.get(b"MediaBox").and_then(Object::as_array) {
            let values: Vec<f64> = array.iter().filter_map(|o| o.as_float().ok().map(f64::from).or_else(|| o.as_i64().ok().map(|v| v as f64))).collect();
            if values.len() == 4 { return [values[0], values[1], values[2], values[3]]; }
        }
        node = dict.get(b"Parent").and_then(Object::as_reference).ok();
    }
    [0.0, 0.0, 612.0, 792.0]
}

type Matriz = [f64; 6];
const IDENTIDADE: Matriz = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
/// m1 × m2 (convenção do PDF: vetor-linha à esquerda).
fn mult(m1: Matriz, m2: Matriz) -> Matriz {
    [
        m1[0] * m2[0] + m1[1] * m2[2], m1[0] * m2[1] + m1[1] * m2[3],
        m1[2] * m2[0] + m1[3] * m2[2], m1[2] * m2[1] + m1[3] * m2[3],
        m1[4] * m2[0] + m1[5] * m2[2] + m2[4], m1[4] * m2[1] + m1[5] * m2[3] + m2[5],
    ]
}
fn ponto(m: Matriz, x: f64, y: f64) -> (f64, f64) {
    (x * m[0] + y * m[2] + m[4], x * m[1] + y * m[3] + m[5])
}
fn numeros(operands: &[lopdf::Object]) -> Vec<f64> {
    operands.iter().filter_map(|o| o.as_float().ok().map(f64::from).or_else(|| o.as_i64().ok().map(|v| v as f64))).collect()
}

/// Tira do conteúdo os comandos de texto que começam dentro das caixas (o valor antigo some
/// também da cópia e da busca, não só da tela). Segue a posição pelo `cm`, `Tm`, `Td`, `TD`, `T*`
/// e `TL`, a partir de `ctm`. Devolve o conteúdo filtrado, quantos comandos saíram e os desenhos
/// de objetos (`Do`) com a posição de cada um, para entrar neles também.
fn sem_texto(conteudo: &[u8], caixas: &[[f64; 4]], mbox: [f64; 4], ctm: Matriz) -> Option<(Vec<u8>, usize, Vec<(Vec<u8>, Matriz)>)> {
    use lopdf::content::{Content, Operation};
    let decoded = Content::decode(conteudo).ok()?;
    let (mut ctm, mut stack) = (ctm, Vec::<Matriz>::new());
    let (mut tm, mut tlm, mut leading, mut size) = (IDENTIDADE, IDENTIDADE, 0.0, 10.0);
    let mut kept: Vec<Operation> = Vec::with_capacity(decoded.operations.len());
    let (mut removed, mut draws) = (0, Vec::new());
    for op in decoded.operations {
        let n = numeros(&op.operands);
        match op.operator.as_str() {
            "q" => stack.push(ctm),
            "Q" => ctm = stack.pop().unwrap_or(ctm),
            "cm" if n.len() == 6 => ctm = mult([n[0], n[1], n[2], n[3], n[4], n[5]], ctm),
            "BT" => { tm = IDENTIDADE; tlm = IDENTIDADE; }
            "Tm" if n.len() == 6 => { tm = [n[0], n[1], n[2], n[3], n[4], n[5]]; tlm = tm; }
            "Td" if n.len() == 2 => { tlm = mult([1.0, 0.0, 0.0, 1.0, n[0], n[1]], tlm); tm = tlm; }
            "TD" if n.len() == 2 => { leading = -n[1]; tlm = mult([1.0, 0.0, 0.0, 1.0, n[0], n[1]], tlm); tm = tlm; }
            "TL" if n.len() == 1 => leading = n[0],
            "Tf" if !n.is_empty() => size = n[n.len() - 1],
            "T*" | "'" | "\"" => { tlm = mult([1.0, 0.0, 0.0, 1.0, 0.0, -leading], tlm); tm = tlm; }
            "Do" => if let Some(Ok(name)) = op.operands.first().map(lopdf::Object::as_name) { draws.push((name.to_vec(), ctm)); },
            _ => {}
        }
        if matches!(op.operator.as_str(), "Tj" | "TJ" | "'" | "\"") {
            // Um ponto dentro da primeira letra (um pouco acima da linha de base).
            let (x, y) = ponto(mult(tm, ctm), size * 0.25, size * 0.3);
            if caixas.iter().any(|c| dentro(*c, (x - mbox[0], mbox[3] - y))) {
                removed += 1;
                continue;
            }
        }
        kept.push(op);
    }
    Some((Content { operations: kept }.encode().ok()?, removed, draws))
}

/// Dicionário (direto ou por referência).
fn dict_of(doc: &lopdf::Document, object: Option<&lopdf::Object>) -> lopdf::Dictionary {
    match object {
        Some(lopdf::Object::Dictionary(d)) => d.clone(),
        Some(lopdf::Object::Reference(r)) => doc.get_dictionary(*r).cloned().unwrap_or_default(),
        _ => lopdf::Dictionary::new(),
    }
}

/// Tira o texto antigo de um conteúdo e dos objetos de desenho que ele usa (o Canva põe a página
/// inteira dentro deles). Objetos mudados viram cópias novas (podem ser usados em outras páginas).
/// Devolve o conteúdo novo e os recursos novos, ou `None` se nada mudou.
fn limpar(doc: &mut lopdf::Document, conteudo: &[u8], resources: &lopdf::Dictionary, ctm: Matriz, caixas: &[[f64; 4]], mbox: [f64; 4], depth: u32) -> Option<(Vec<u8>, lopdf::Dictionary)> {
    use lopdf::{Object, Stream};
    let (content, removed, draws) = sem_texto(conteudo, caixas, mbox, ctm)?;
    let mut xobjects = dict_of(doc, resources.get(b"XObject").ok());
    let mut changed_x = false;
    if depth < 8 {
        for (name, at) in draws {
            let Ok(Object::Reference(id)) = xobjects.get(&name).cloned() else { continue };
            let Ok(stream) = doc.get_object(id).and_then(Object::as_stream).cloned() else { continue };
            if stream.dict.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Form") { continue; }
            let matrix = stream.dict.get(b"Matrix").and_then(Object::as_array).ok()
                .map(|a| numeros(a)).filter(|n| n.len() == 6).map(|n| [n[0], n[1], n[2], n[3], n[4], n[5]]).unwrap_or(IDENTIDADE);
            let inner = stream.decompressed_content().unwrap_or_else(|_| stream.content.clone());
            let form_resources = dict_of(doc, stream.dict.get(b"Resources").ok());
            let Some((new_content, new_resources)) = limpar(doc, &inner, &form_resources, mult(matrix, at), caixas, mbox, depth + 1) else { continue };
            let mut dict = stream.dict.clone();
            dict.remove(b"Filter");
            dict.remove(b"DecodeParms");
            dict.remove(b"Length");
            dict.set("Resources", Object::Dictionary(new_resources));
            let mut copy = Stream::new(dict, new_content);
            let _ = copy.compress();
            let new_id = doc.add_object(Object::Stream(copy));
            xobjects.set(name, Object::Reference(new_id));
            changed_x = true;
        }
    }
    if removed == 0 && !changed_x { return None; }
    let mut resources = resources.clone();
    if changed_x { resources.set("XObject", Object::Dictionary(xobjects)); }
    Some((content, resources))
}

/// Grava em `destino` o PDF `original` com as camadas das fichas. As outras páginas e todo o
/// conteúdo original continuam iguais, menos o texto antigo dentro dos campos; a camada fica num
/// objeto à parte, por cima.
pub fn aplicar(original: &Path, destino: &Path, fichas: &[Ficha]) -> Result<(), String> {
    use lopdf::{Dictionary, Object, Stream};
    let mut doc = lopdf::Document::load(original).map_err(|_| "Não foi possível abrir o PDF da ficha".to_string())?;
    if doc.is_encrypted() {
        return Err("PDFs protegidos com senha não podem ser editados".into());
    }
    let pages = doc.get_pages();
    // Por página do PDF: os campos que mudaram em relação ao original. Os iguais ficam como o PDF
    // os desenhou (fonte, alinhamento e tamanho originais); sem leitura do original, todos mudam.
    let mut by_page: BTreeMap<u32, Vec<(&Campo, Option<&Valor>)>> = BTreeMap::new();
    let mut todos: BTreeMap<u32, Vec<&Campo>> = BTreeMap::new();
    for ficha in fichas {
        let originais = ler(ficha.mapa, original, ficha.primeira).ok();
        for campo in &ficha.mapa.campos {
            todos.entry(ficha.primeira + campo.pagina).or_default().push(campo);
            let valor = ficha.valores.get(&campo.id);
            if originais.as_ref().is_some_and(|o| o.get(&campo.id) == valor) {
                continue;
            }
            by_page.entry(ficha.primeira + campo.pagina).or_default().push((campo, valor));
        }
    }
    // 1) Sem o texto antigo dos campos (o conteúdo original fica entre q … Q).
    let mut boxes: BTreeMap<u32, (lopdf::ObjectId, [f64; 4])> = BTreeMap::new();
    for (number, campos) in &by_page {
        let page = *pages.get(number).ok_or("A ficha passa do fim do PDF")?;
        let mbox = media_box(&doc, page);
        boxes.insert(*number, (page, mbox));
        let caixas: Vec<[f64; 4]> = campos.iter().filter(|(c, _)| !matches!(c.tipo, Tipo::Bolinhas { .. })).map(|(c, _)| c.caixa).collect();
        let current = doc.get_page_content(page).map_err(lopdf_error)?;
        let page_resources = recursos(&doc, page)?;
        let (content, resources) = limpar(&mut doc, &current, &page_resources, IDENTIDADE, &caixas, mbox, 0).unwrap_or((current, page_resources));
        doc.get_object_mut(page).and_then(Object::as_dict_mut).map_err(lopdf_error)?.set("Resources", Object::Dictionary(resources));
        let mut body = b"q\n".to_vec();
        body.extend(content);
        body.extend(b"\nQ\n");
        let mut stream = Stream::new(Dictionary::new(), body);
        let _ = stream.compress();
        let id = doc.add_object(Object::Stream(stream));
        doc.get_object_mut(page).and_then(Object::as_dict_mut).map_err(lopdf_error)?.set("Contents", Object::Reference(id));
    }
    // 2) O que ainda sobrou de texto nos campos (texto dentro de objetos, por exemplo) é coberto.
    let work = Temp::new()?;
    let stripped = work.0.join("sem-texto.pdf");
    doc.save(&stripped).map_err(|_| "Não foi possível preparar o PDF da ficha".to_string())?;
    let mut font = Dictionary::new();
    font.set("Type", Object::Name(b"Font".to_vec()));
    font.set("Subtype", Object::Name(b"Type1".to_vec()));
    font.set("BaseFont", Object::Name(b"Helvetica-Bold".to_vec()));
    font.set("Encoding", Object::Name(b"WinAnsiEncoding".to_vec()));
    let font_id = doc.add_object(Object::Dictionary(font));
    for (number, campos) in by_page {
        let (page, mbox) = boxes[&number];
        let texto = palavras(&stripped, number)?;
        let pixels = imagem(original, number)?;
        let antes = palavras(original, number)?;
        let ops = camada(&campos, &todos[&number], &texto, &antes, &pixels, mbox[3], mbox[0]);
        let mut form = Dictionary::new();
        form.set("Type", Object::Name(b"XObject".to_vec()));
        form.set("Subtype", Object::Name(b"Form".to_vec()));
        form.set("BBox", Object::Array(mbox.iter().map(|v| Object::Real(*v as f32)).collect()));
        let mut fonts = Dictionary::new();
        fonts.set("AylaF", Object::Reference(font_id));
        let mut form_resources = Dictionary::new();
        form_resources.set("Font", Object::Dictionary(fonts));
        form.set("Resources", Object::Dictionary(form_resources));
        let mut stream = Stream::new(form, ops.into_bytes());
        let _ = stream.compress();
        let form_id = doc.add_object(Object::Stream(stream));
        // Recursos próprios da página com o XObject da camada (sem perder os herdados).
        let mut resources = recursos(&doc, page)?;
        let mut xobjects = match resources.get(b"XObject") {
            Ok(Object::Dictionary(d)) => d.clone(),
            Ok(Object::Reference(r)) => doc.get_dictionary(*r).cloned().unwrap_or_default(),
            _ => Dictionary::new(),
        };
        // Nome novo (o PDF pode já ter a camada de uma edição anterior, feita fora do original).
        let mut nome = String::from("AylaFicha");
        let mut n = 2;
        while xobjects.has(nome.as_bytes()) { nome = format!("AylaFicha{n}"); n += 1; }
        xobjects.set(nome.as_bytes().to_vec(), Object::Reference(form_id));
        resources.set("XObject", Object::Dictionary(xobjects));
        let after = doc.add_object(Object::Stream(Stream::new(Dictionary::new(), format!("q /{nome} Do Q\n").into_bytes())));
        let dict = doc.get_object_mut(page).and_then(Object::as_dict_mut).map_err(lopdf_error)?;
        let base = dict.get(b"Contents").ok().cloned().ok_or("Página sem conteúdo")?;
        dict.set("Resources", Object::Dictionary(resources));
        dict.set("Contents", Object::Array(vec![base, Object::Reference(after)]));
    }
    doc.prune_objects();
    let temporary = destino.with_extension(format!("ayla-{}.tmp", super::store::id()));
    doc.save(&temporary).map_err(|_| "Não foi possível gravar o PDF da ficha".to_string())?;
    fs::rename(&temporary, destino).map_err(|_| { let _ = fs::remove_file(&temporary); "Não foi possível substituir o PDF da ficha".to_string() })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ayla-fichas-{name}-{}", super::super::store::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub(crate) fn poppler() -> bool {
        Command::new("pdftoppm").arg("-v").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok()
    }

    /// Ficha de teste no estilo das do Canva: fundo cinza-claro, células brancas com borda escura,
    /// rótulos em maiúsculas e alguns valores preenchidos (número, linha, traços, bolinhas).
    pub(crate) fn ficha_de_teste(path: &Path, preenchida: bool) {
        use lopdf::{Dictionary, Object, Stream};
        let h = 400.0;
        let mut ops = String::from("0.94 0.93 0.86 rg 0 0 300 400 re f\n");
        // Células: (x, y do topo, largura, altura), origem em cima.
        let cells = [
            (10.0, 10.0, 280.0, 18.0),   // faixa NOME
            (10.0, 30.0, 280.0, 22.0),   // valor do nome
            (10.0, 60.0, 80.0, 20.0),    // célula PODEROSO
            (92.0, 60.0, 20.0, 20.0), (114.0, 60.0, 20.0, 20.0), (136.0, 60.0, 20.0, 20.0), // bolinhas
            (170.0, 60.0, 80.0, 20.0),   // célula GOLPEAR
            (262.0, 60.0, 28.0, 20.0),   // número de golpear
            (10.0, 100.0, 280.0, 24.0),  // faixa TRAÇOS
            (10.0, 126.0, 280.0, 160.0), // área dos traços
        ];
        for (x, y, w, hh) in cells {
            ops.push_str(&format!("0.1 0.1 0.1 rg {} {} {} {} re f 1 1 1 rg {} {} {} {} re f\n", x - 1.5, h - y - hh - 1.5, w + 3.0, hh + 3.0, x, h - y - hh, w, hh));
        }
        // Ícone escuro entre GOLPEAR e o número (como o ⊕).
        ops.push_str(&format!("0.1 0.1 0.1 rg 252 {} 8 8 re f\n", h - 74.0));
        let mut text = |s: &str, x: f64, y_top: f64, size: f64| ops.push_str(&format!("BT 0 0 0.6 rg /F1 {size} Tf {x} {} Td {} Tj ET\n", h - y_top, pdf_string(s)));
        text("NOME", 14.0, 24.0, 10.0);
        text("PODEROSO", 14.0, 74.0, 9.0);
        text("GOLPEAR", 174.0, 74.0, 9.0);
        text("TRAÇOS", 120.0, 118.0, 12.0);
        if preenchida {
            text("BARTHOLOMEW", 14.0, 46.0, 10.0);
            text("3", 272.0, 74.0, 10.0);
            text("DETERMINAÇÃO", 100.0, 145.0, 11.0);
            text("VONTADE DE VIVER", 100.0, 158.0, 8.0);
            ops.push_str(&format!("0 0 0 rg 98 {} 8 8 re f 120 {} 8 8 re f\n", h - 74.0, h - 74.0));
        }
        let mut doc = lopdf::Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let mut font = Dictionary::new();
        font.set("Type", Object::Name(b"Font".to_vec()));
        font.set("Subtype", Object::Name(b"Type1".to_vec()));
        font.set("BaseFont", Object::Name(b"Helvetica-Bold".to_vec()));
        font.set("Encoding", Object::Name(b"WinAnsiEncoding".to_vec()));
        let font_id = doc.add_object(Object::Dictionary(font));
        let content = doc.add_object(Object::Stream(Stream::new(Dictionary::new(), ops.into_bytes())));
        let mut page = Dictionary::new();
        page.set("Type", Object::Name(b"Page".to_vec()));
        page.set("Parent", Object::Reference(pages_id));
        page.set("Contents", Object::Reference(content));
        let page_id = doc.add_object(Object::Dictionary(page));
        // Recursos herdados da árvore de páginas (o caso que não pode se perder).
        let mut fonts = Dictionary::new();
        fonts.set("F1", Object::Reference(font_id));
        let mut resources = Dictionary::new();
        resources.set("Font", Object::Dictionary(fonts));
        let mut pages = Dictionary::new();
        pages.set("Type", Object::Name(b"Pages".to_vec()));
        pages.set("Kids", Object::Array(vec![Object::Reference(page_id)]));
        pages.set("Count", Object::Integer(1));
        pages.set("Resources", Object::Dictionary(resources));
        pages.set("MediaBox", Object::Array(vec![0.into(), 0.into(), 300.into(), 400.into()]));
        doc.objects.insert(pages_id, Object::Dictionary(pages));
        let mut catalog = Dictionary::new();
        catalog.set("Type", Object::Name(b"Catalog".to_vec()));
        catalog.set("Pages", Object::Reference(pages_id));
        let catalog_id = doc.add_object(Object::Dictionary(catalog));
        doc.trailer.set("Root", Object::Reference(catalog_id));
        doc.save(path).unwrap();
    }

    /// Ficha real: `AYLA_FICHA_BRANCA` (modelo), `AYLA_FICHA_CHEIA` (preenchida) e `AYLA_FICHA_SAIDA`.
    #[test]
    #[ignore = "usa uma ficha real indicada por variáveis de ambiente"]
    fn ficha_real() {
        let var = |k: &str| PathBuf::from(std::env::var(k).unwrap_or_else(|_| panic!("{k}")));
        let (branca, cheia, saida) = (var("AYLA_FICHA_BRANCA"), var("AYLA_FICHA_CHEIA"), var("AYLA_FICHA_SAIDA"));
        let mut campos = Vec::new();
        for page in 0..2 {
            let t = std::time::Instant::now();
            let found = propor(&palavras(&branca, page + 1).unwrap(), &imagem(&branca, page + 1).unwrap(), page);
            eprintln!("página {} proposta em {:?}", page + 1, t.elapsed());
            campos.extend(found);
        }
        for c in &campos { eprintln!("{:>2} {:28} {:?} {:?}", c.pagina, c.rotulo, c.tipo, c.caixa); }
        let mapa = Mapa { nome: "Wilderfeast".into(), paginas: 2, campos };
        let t = std::time::Instant::now();
        let valores = ler(&mapa, &cheia, 1).unwrap();
        eprintln!("lida em {:?}", t.elapsed());
        for (k, v) in &valores { eprintln!("{k} = {v:?}"); }
        let mut novos = valores.clone();
        if let Some(Valor::Numero(n)) = valores.get("golpear") { novos.insert("golpear".into(), Valor::Numero(n + 1)); }
        let t = std::time::Instant::now();
        aplicar(&cheia, &saida, &[Ficha { mapa: &mapa, valores: &novos, primeira: 1 }]).unwrap();
        eprintln!("gravada em {:?}", t.elapsed());
        let relidos = ler(&mapa, &saida, 1).unwrap();
        for (k, v) in &novos { if relidos.get(k) != Some(v) { eprintln!("DIFERENTE {k}: {v:?} → {:?}", relidos.get(k)); } }
    }

    #[test]
    fn separa_fichas_modelo_em_branco_e_nomes() {
        let set = |w: &[&str]| w.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
        let um = ["NOME", "PRONOMES", "FERRAMENTA", "ESTILOS", "HABILIDADES", "GOLPEAR"];
        let dois = ["NOME", "PRONOMES", "VOCÊ", "FAMILIAR MONSTRUOSO", "TEMPERO", "ANTECEDENTES"];
        let com = |base: &[&str], extra: &str| { let mut s = set(base); s.insert(extra.into()); s };
        // Três personagens de duas páginas: o nome (em maiúsculas) também vira "rótulo".
        let pdf = [com(&um, "BART"), com(&dois, "BART"), com(&um, "KAEL"), com(&dois, "KAEL"), com(&um, "AURORA"), com(&dois, "AURORA")];
        assert_eq!(separar(&pdf), (2, vec![1, 3, 5]));
        assert_eq!(separar(&[com(&um, "X")]), (1, vec![1]));
        assert_eq!(separar(&[com(&um, "X"), com(&dois, "X")]), (2, vec![1]), "uma ficha: o PDF inteiro");
        let branco = [set(&um), set(&dois)];
        assert_eq!(inicios_do_modelo(&branco, &pdf), vec![1, 3, 5]);
        assert!(combina(&branco, &pdf, 1) < 0.5, "a ficha em branco não começa na página 2");
        // Palavras comuns às fichas: os valores de cada uma saem.
        let w = |t: &str| Palavra { texto: t.into(), caixa: [0.0, 0.0, 1.0, 1.0] };
        let pagina = |extra: &str| Pagina { largura: 10.0, altura: 10.0, palavras: vec![w("NOME"), w(extra), w("GOLPEAR")] };
        let (a, b) = (pagina("BART"), pagina("KAEL"));
        assert_eq!(comum(&[&a, &b]).unwrap().palavras.iter().map(|p| p.texto.as_str()).collect::<Vec<_>>(), ["NOME", "GOLPEAR"]);
        // Nome pelo pedido: começo do nome com 4 letras ou mais, palavra inteira com 3.
        assert_eq!(nome_bate("Bartholomew Roberts", "adiciona um ponto na ficha do bart"), 1);
        assert_eq!(nome_bate("Bartholomew Roberts", "ficha do Bartholomew Roberts"), 2);
        assert_eq!(nome_bate("Kael Kûarasy", "ficha do bart"), 0);
        assert_eq!(nome_bate("Bohr", "ficha do bo"), 0);
        assert_eq!(nome_bate("Bohr", "ficha do Bohr"), 1);
    }

    #[test]
    fn rotulos_slug_titulo_e_texto_do_pdf() {
        let w = |t: &str, x0: f64, x1: f64, y: f64| Palavra { texto: t.into(), caixa: [x0, y, x1, y + 10.0] };
        let labels = rotulos(&[w("MAS", 10.0, 30.0, 5.0), w("LUTA", 33.0, 55.0, 5.0), w("ALCANCE:", 10.0, 50.0, 40.0), w("ESPADA", 53.0, 90.0, 40.0), w("valor", 10.0, 30.0, 70.0)]);
        assert_eq!(labels.iter().map(|l| l.texto.as_str()).collect::<Vec<_>>(), ["MAS LUTA", "ALCANCE", "ESPADA"]);
        assert_eq!(slug("Vigor · Atual"), "vigor-atual");
        assert_eq!(slug("CONDIÇÕES"), "condicoes");
        assert_eq!(titulo("MAS LUTA PRA SER"), "Mas luta pra ser");
        assert_eq!(winansi("Ação (1)€"), b"A\xe7\xe3o (1)\x80");
        assert_eq!(pdf_string("a(b)\\ç"), "(a\\(b\\)\\\\\\347)");
        assert!((medir("A", 10.0) - 7.22).abs() < 0.01);
        assert_eq!(quebrar("um dois tres", 10.0, medir("um dois", 10.0)), ["um dois", "tres"]);
        let page = parse_bbox(r#"<page width="300.5" height="400"><word xMin="1" yMin="2" xMax="3" yMax="4">A&amp;B</word></page>"#).unwrap();
        assert_eq!((page.largura, page.palavras[0].texto.as_str(), page.palavras[0].caixa), (300.5, "A&B", [1.0, 2.0, 3.0, 4.0]));
    }

    #[test]
    fn celulas_claras_e_ppm() {
        // 60×40 px escuros com uma célula branca de 8×4 em (1, 1) e uma área clara embaixo que
        // encosta na borda da página (é o fundo, não célula).
        let mut pixels = vec![0u8; 60 * 40 * 3];
        let mut paint = |x0: usize, y0: usize, x1: usize, y1: usize| for y in y0..y1 { for x in x0..x1 { let i = (y * 60 + x) * 3; pixels[i..i + 3].copy_from_slice(&[255, 255, 255]); } };
        paint(1, 1, 9, 5);
        paint(0, 10, 60, 40);
        let image = Imagem::from_rgb(60, 40, 1.0, pixels.clone());
        assert_eq!(image.celula(3.0, 2.0), Some([1.0, 1.0, 9.0, 5.0]));
        assert_eq!(image.celula(0.0, 0.0), None);
        assert_eq!(image.celula(5.0, 20.0), None);
        let mut ppm = b"P6\n# comentario\n60 40\n255\n".to_vec();
        ppm.extend(&pixels);
        assert_eq!(Imagem::from_ppm(&ppm, 1.0).unwrap().celula(3.0, 2.0), Some([1.0, 1.0, 9.0, 5.0]));
    }

    #[test]
    fn propoe_le_e_desenha_uma_ficha() {
        if !poppler() { eprintln!("sem poppler-utils: teste pulado"); return; }
        let dir = temp("ciclo");
        let (branca, cheia, saida) = (dir.join("branca.pdf"), dir.join("cheia.pdf"), dir.join("saida.pdf"));
        ficha_de_teste(&branca, false);
        ficha_de_teste(&cheia, true);
        let campos = propor(&palavras(&branca, 1).unwrap(), &imagem(&branca, 1).unwrap(), 0);
        let find = |id: &str| campos.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("{id}: {campos:?}"));
        assert_eq!(find("nome").tipo, Tipo::Linha);
        assert!(find("nome").caixa[1] >= 29.0, "nome abaixo da faixa: {:?}", find("nome"));
        assert_eq!(find("poderoso").tipo, Tipo::Bolinhas { maximo: 3 });
        assert_eq!(find("golpear").tipo, Tipo::Numero);
        assert!(find("golpear").caixa[0] >= 261.0, "número depois do ícone: {:?}", find("golpear"));
        assert_eq!(find("tracos").tipo, Tipo::Bloco, "{:?}", find("tracos"));
        let mut mapa = Mapa { nome: "Teste".into(), paginas: 1, campos: campos.clone() };
        mapa.campos.iter_mut().find(|c| c.id == "tracos").unwrap().tipo = Tipo::Tracos;
        // Ficha preenchida: lê os valores.
        let lidos = ler(&mapa, &cheia, 1).unwrap();
        assert_eq!(lidos["nome"], Valor::Texto("BARTHOLOMEW".into()));
        assert_eq!(lidos["golpear"], Valor::Numero(3));
        assert_eq!(lidos["poderoso"], Valor::Numero(2));
        assert_eq!(lidos["tracos"], Valor::Tracos(vec![Traco { titulo: "DETERMINAÇÃO".into(), descricao: "VONTADE DE VIVER".into() }]));
        // Muda e desenha: o PDF novo tem os valores novos e não tem mais os antigos.
        let mut novos = lidos.clone();
        novos.insert("golpear".into(), Valor::Numero(4));
        novos.insert("poderoso".into(), Valor::Numero(3));
        novos.insert("nome".into(), Valor::Texto("BART ROBERTS".into()));
        novos.insert("tracos".into(), Valor::Tracos(vec![Traco { titulo: "DETERMINAÇÃO".into(), descricao: "VONTADE DE VIVER".into() }, Traco { titulo: "FÔLEGO".into(), descricao: "AGUENTA MAIS".into() }]));
        aplicar(&cheia, &saida, &[Ficha { mapa: &mapa, valores: &novos, primeira: 1 }]).unwrap();
        let relidos = ler(&mapa, &saida, 1).unwrap();
        assert_eq!(relidos, novos, "a releitura do PDF novo devolve os valores novos");
        // O resto da página continua: rótulos com a fonte herdada da árvore de páginas.
        let texto = palavras(&saida, 1).unwrap();
        for label in ["NOME", "PODEROSO", "GOLPEAR", "TRAÇOS"] {
            assert!(texto.palavras.iter().any(|p| p.texto == label), "{label} sumiu");
        }
        // Só o que mudou é redesenhado: o nome igual continua com a posição e a fonte originais.
        let mut so_golpear = lidos.clone();
        so_golpear.insert("golpear".into(), Valor::Numero(5));
        aplicar(&cheia, &saida, &[Ficha { mapa: &mapa, valores: &so_golpear, primeira: 1 }]).unwrap();
        assert_eq!(ler(&mapa, &saida, 1).unwrap(), so_golpear);
        let caixa_do = |pdf: &Path, word: &str| palavras(pdf, 1).unwrap().palavras.into_iter().find(|p| p.texto == word).map(|p| p.caixa);
        assert_eq!(caixa_do(&saida, "BARTHOLOMEW"), caixa_do(&cheia, "BARTHOLOMEW"), "o nome igual não é redesenhado");
        // O número novo segue o tamanho e o centro do antigo.
        let (antigo, novo) = (caixa_do(&cheia, "3").unwrap(), caixa_do(&saida, "5").unwrap());
        assert!((altura(antigo) - altura(novo)).abs() < 1.0, "altura {antigo:?} → {novo:?}");
        assert!((centro(antigo).0 - centro(novo).0).abs() < 1.0, "centro {antigo:?} → {novo:?}");
        fs::remove_dir_all(dir).unwrap();
    }
}
