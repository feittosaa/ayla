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
    /// Linhas pontilhadas cinza que atravessam a caixa (separadores dos traços): a altura de cada
    /// uma, em pontos. Linha de texto não conta (tem pixels escuros; a pontilhada é só cinza).
    pub fn linhas_pontilhadas(&self, caixa: [f64; 4]) -> Vec<f64> {
        let s = self.escala;
        let (x0, x1) = ((caixa[0] * s).max(0.0) as usize, ((caixa[2] * s) as usize).min(self.largura));
        let (y0, y1) = ((caixa[1] * s).max(0.0) as usize, ((caixa[3] * s) as usize).min(self.altura));
        if x1 <= x0 + 10 { return Vec::new(); }
        let largura = (x1 - x0) as f64;
        let mut linhas: Vec<(usize, usize)> = Vec::new();
        for y in y0..y1 {
            let (mut cinza, mut escuro, mut primeiro, mut ultimo) = (0usize, 0usize, usize::MAX, 0usize);
            for x in x0..x1 {
                let [r, g, b] = self.rgb(x, y);
                let media = (r as u32 + g as u32 + b as u32) / 3;
                let neutro = r.abs_diff(g) < 30 && g.abs_diff(b) < 30;
                if media < 110 { escuro += 1; } else if media < 232 && neutro { cinza += 1; primeiro = primeiro.min(x); ultimo = x; }
            }
            let linha = cinza as f64 > largura * 0.2 && (ultimo.saturating_sub(primeiro)) as f64 > largura * 0.6 && (escuro as f64) < largura * 0.02;
            if !linha { continue; }
            match linhas.last_mut() { Some((_, fim)) if *fim + 1 >= y => *fim = y, _ => linhas.push((y, y)) }
        }
        linhas.into_iter().filter(|(a, b)| b - a <= (3.0 * s) as usize).map(|(a, b)| (a + b) as f64 / 2.0 / s).collect()
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
    // "CARANGUEJO DE LAVA" → "Caranguejo de Lava": conectores minúsculos (menos no começo).
    let menores = ["de", "da", "do", "das", "dos", "e", "a", "o", "em", "na", "no"];
    text.split_whitespace().enumerate().map(|(i, w)| if i > 0 && menores.contains(&w.to_lowercase().as_str()) { w.to_lowercase() } else { titulo(w) }).collect::<Vec<_>>().join(" ")
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
    // Valores em maiúsculas já preenchidos ("BESTA ARK" sob "NOME") que não são rótulos: um único
    // texto, sem ":", numa área de uma linha logo abaixo do rótulo.
    let mut valores: BTreeSet<usize> = BTreeSet::new();
    let valor_sob = |area: [f64; 4], i: usize| -> Option<usize> {
        let label = &labels[i];
        let inside: Vec<usize> = labels.iter().enumerate().filter(|(j, l)| *j != i && dentro(area, centro(l.caixa))).map(|(j, _)| j).collect();
        match inside.as_slice() {
            [j] if altura(area) <= altura(label.caixa) * 2.8 && centro(labels[*j].caixa).1 > centro(label.caixa).1 + altura(label.caixa) * 0.5 && !labels[*j].texto.ends_with(':') => Some(*j),
            _ => None,
        }
    };
    for (i, label) in labels.iter().enumerate() {
        if valores.contains(&i) { continue; }
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
        // Rótulo com dois-pontos ("Atual:", "Alcance:"): o valor vem na mesma linha, nunca embaixo.
        let colon = pagina.palavras.iter().any(|w| w.texto.ends_with(':') && (w.caixa[2] - label.caixa[2]).abs() < 0.6 && (centro(w.caixa).1 - cy).abs() < h * 0.5);
        // Faixa de título: a célula do rótulo é baixa, ou logo abaixo há outra da mesma largura e
        // sem rótulo (o valor vai embaixo, como em Nome | Pronomes | Ferramenta).
        let strip = !colon && own.is_some_and(|c| altura(c) < h * 1.65 || below(c).is_some_and(|b| {
            (largura(b) - largura(c)).abs() < largura(c) * 0.1 && b[1] - c[3] < 6.0 && altura(b) >= altura(c) * 0.8 && (!has_label(b, i) || (neighbors.is_empty() && valor_sob(b, i).is_some()))
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
        let Some(area) = below(base).or_else(|| below(label.caixa)) else {
            // Cabeçalho dentro da própria coluna em branco (Vigor: "Atual | Max" em cima dela): o
            // campo é a coluna abaixo do rótulo, dividida entre os rótulos da mesma linha.
            let Some(cell) = imagem.celula(centro(label.caixa).0, label.caixa[3] + 2.0).filter(|c| c[1] <= label.caixa[1] + 1.0 && c[3] > label.caixa[3] + h) else { continue };
            let mut row: Vec<usize> = labels.iter().enumerate()
                .filter(|(_, l)| (centro(l.caixa).1 - cy).abs() < h * 0.5 && centro(l.caixa).0 > cell[0] && centro(l.caixa).0 < cell[2]).map(|(j, _)| j).collect();
            row.sort_by(|a, b| labels[*a].caixa[0].partial_cmp(&labels[*b].caixa[0]).unwrap_or(std::cmp::Ordering::Equal));
            let pos = row.iter().position(|&j| j == i).unwrap_or(0);
            let left = if pos == 0 { cell[0] } else { (labels[row[pos - 1]].caixa[2] + label.caixa[0]) / 2.0 };
            let right = if pos + 1 == row.len() { cell[2] } else { (label.caixa[2] + labels[row[pos + 1]].caixa[0]) / 2.0 };
            let caixa = [left, label.caixa[3] + 1.0, right, cell[3]];
            found.push(Proposta { label: i, caixa, tipo: if altura(caixa) > h * 2.8 { Tipo::Bloco } else { Tipo::Linha } });
            continue;
        };
        let inside: Vec<usize> = labels.iter().enumerate().filter(|(j, l)| *j != i && dentro(area, centro(l.caixa))).map(|(j, _)| j).collect();
        // Ficha preenchida: um único texto em maiúsculas numa área de uma linha logo abaixo do
        // rótulo é o valor dele (o nome da fera), não outro rótulo.
        let valor = valor_sob(area, i);
        if inside.is_empty() || valor.is_some() {
            if let Some(j) = valor { valores.insert(j); }
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
    let campos_de: BTreeSet<usize> = found.iter().map(|p| p.label).collect();
    let count = |text: &str| repeated.iter().filter(|t| *t == text).count();
    let mut used: Vec<String> = Vec::new();
    found.into_iter().map(|p| {
        let label = &labels[p.label];
        let section = sections.iter().filter(|(_, area)| dentro(*area, centro(label.caixa)))
            .min_by(|a, b| (largura(a.1) * altura(a.1)).partial_cmp(&(largura(b.1) * altura(b.1))).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(s, _)| titulo(&labels[*s].texto));
        let mut rotulo = titulo(&label.texto);
        if count(&label.texto) > 1 {
            // "Durabilidade  Atual: [ ]  Max: [ ]": o rótulo sem campo à esquerda, na mesma linha, dá o nome.
            let (_, cy) = centro(label.caixa);
            let dono = labels.iter().enumerate()
                .filter(|(j, l)| !campos_de.contains(j) && (centro(l.caixa).1 - cy).abs() < altura(label.caixa) * 0.5 && l.caixa[2] <= label.caixa[0] && label.caixa[0] - l.caixa[2] < 200.0)
                .max_by(|a, b| a.1.caixa[2].partial_cmp(&b.1.caixa[2]).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(_, l)| titulo(&l.texto));
            rotulo = match dono.or(section) { Some(s) => format!("{s} · {rotulo}"), None => rotulo };
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

/// Pedido de mudança sem dizer "ficha" ("altera o nome dele pra…, coloca 50 de vida"): vale só
/// logo depois de uma resposta sobre uma ficha.
pub fn pede_mudanca(message: &str) -> bool {
    let verbos = ["altera", "altere", "alterar", "muda", "mude", "mudar", "coloca", "coloque", "colocar", "poe", "bota", "tira", "tire", "tirar",
        "remove", "remova", "remover", "adiciona", "adicione", "adicionar", "acrescenta", "acrescente", "troca", "troque", "trocar",
        "aumenta", "aumente", "diminui", "diminua", "renomeia", "renomeie", "substitui", "substitua", "deixa", "deixe", "preenche", "preencha"];
    slug(message).split('-').any(|w| verbos.contains(&w))
}

/// Campos que se reconhecem pelo nome: "Traços" é uma lista (título maior e descrição, centralizados),
/// não um texto corrido — assim a Ayla tira e põe traços pelo título e desenha como a ficha.
pub fn tipos_conhecidos(campos: &mut [Campo]) {
    for campo in campos.iter_mut() {
        if campo.tipo == Tipo::Bloco && slug(&campo.rotulo).split('-').last() == Some("tracos") { campo.tipo = Tipo::Tracos; }
    }
}

/// O valor serve para o tipo do campo (numa troca de mapa, o que não serve é lido de novo).
pub fn serve(campo: &Campo, valor: &Valor) -> bool {
    matches!((&campo.tipo, valor), (Tipo::Numero | Tipo::Bolinhas { .. }, Valor::Numero(_)) | (Tipo::Linha | Tipo::Bloco, Valor::Texto(_)) | (Tipo::Tracos, Valor::Tracos(_)))
}

/// Quantos traços do começo ficam quando o pedido manda trocar os outros: "remove os traços exceto
/// pelo primeiro" → 1; "deixa só os dois primeiros" → 2; "substitua os traços" / "tira todos" → 0.
pub fn regra_tracos(message: &str) -> Option<usize> {
    let text = format!(" {} ", slug(message).replace('-', " "));
    if !text.contains(" tracos ") && !text.contains(" traco ") { return None; }
    let numero = |w: &str| match w { "primeiro" | "um" | "1" => Some(1), "dois" | "2" => Some(2), "tres" | "3" => Some(3), _ => None };
    for marker in [" exceto ", " menos ", " so ", " apenas ", " somente ", " fora "] {
        if let Some(at) = text.find(marker) {
            let words: Vec<&str> = text[at + marker.len()..].split_whitespace().take(5).collect();
            if let Some(i) = words.iter().position(|w| w.starts_with("primeir")) {
                // "o primeiro" = 1; "os dois primeiros" = 2.
                return Some(words[..i].iter().rev().find_map(|w| numero(w)).unwrap_or(1));
            }
        }
    }
    // Todos: "substitua os traços", "troca todos os traços", "tira todos". "Troca o segundo traço"
    // é um só (fica com o modelo, que diz qual).
    let todos = text.contains(" todos ") || text.contains(" tudo ");
    let troca = [" substitui ", " substitua ", " substituir ", " troca ", " troque ", " trocar "].iter().any(|m| text.contains(m));
    let tira = [" remove ", " remova ", " tira ", " tire ", " apaga ", " apague "].iter().any(|m| text.contains(m));
    // Trocar "os traços" é trocar todos; tirar só quando diz "todos" ("remove os traços de voo" não).
    ((troca && (todos || text.contains(" os tracos "))) || (tira && todos)).then_some(0)
}

/// O pedido manda criar traços novos ("coloque no lugar habilidades que combinem", "cria 2 traços")
/// e quantos (o número dito perto de "no total", "traços", "habilidades"; sem número, 2).
pub fn pede_tracos_novos(message: &str) -> Option<usize> {
    let words: Vec<String> = slug(message).split('-').map(str::to_string).collect();
    let criar = ["coloque", "coloca", "ponha", "poe", "bota", "crie", "cria", "criar", "adicione", "adiciona", "inventa", "invente", "sugira", "sugere", "gere", "gera", "lugar"];
    let alvo = |w: &str| matches!(w, "traco" | "tracos" | "habilidade" | "habilidades" | "poderes" | "poder");
    if !words.iter().any(|w| criar.contains(&w.as_str())) || !words.iter().any(|w| alvo(w)) { return None; }
    let numero = |w: &str| match w { "um" | "uma" | "1" => Some(1), "dois" | "duas" | "2" => Some(2), "tres" | "3" => Some(3), "quatro" | "4" => Some(4), _ => None };
    let perto = words.iter().enumerate().filter(|(_, w)| alvo(w) || w.as_str() == "total").flat_map(|(i, _)| (i.saturating_sub(3)..(i + 1).min(words.len())).chain(i + 1..(i + 4).min(words.len())))
        .find_map(|j| numero(&words[j]).filter(|_| words[j] != "um" || words.get(j + 1).is_some_and(|w| alvo(w))));
    Some(perto.unwrap_or(2))
}

/// "Coloca 50 de vida", "vigor 50", "PV para 50": o número pedido para o Vigor (sem depender do modelo).
pub fn pedido_vigor(message: &str) -> Option<i64> {
    let words: Vec<String> = slug(message).split('-').map(str::to_string).collect();
    let vida = |w: &str| matches!(w, "vida" | "vigor" | "pv" | "hp");
    let numero = |w: &str| w.parse::<i64>().ok().filter(|n| *n > 0 && *n < 100_000);
    for (i, w) in words.iter().enumerate() {
        if !vida(w) { continue; }
        // Antes: "50 de vida", "50 pontos de vida"; depois: "vida 50", "vida para 50".
        let before = (i.saturating_sub(4)..i).rev().map(|j| &words[j]).take_while(|w| numero(w).is_some() || matches!(w.as_str(), "de" | "pontos" | "ponto")).find_map(|w| numero(w));
        let after = words.iter().skip(i + 1).take(3).take_while(|w| numero(w).is_some() || matches!(w.as_str(), "para" | "pra" | "em" | "de" | "max" | "maximo" | "maxima")).find_map(|w| numero(w));
        if let Some(n) = before.or(after) { return Some(n); }
    }
    None
}

/// "Remove o RD", "tira a RDA", "sem RD": tirar a redução de dano do Vigor.
pub fn pede_sem_rd(message: &str) -> bool {
    let words: Vec<String> = slug(message).split('-').map(str::to_string).collect();
    words.iter().any(|w| w == "rd" || w == "rda")
        && words.iter().any(|w| matches!(w.as_str(), "remove" | "remova" | "remover" | "tira" | "tire" | "tirar" | "retira" | "retire" | "sem" | "apaga" | "apague"))
}

/// Aplica o Vigor pedido e a retirada do RD no campo de Vigor máximo (ou no único de Vigor):
/// troca o número da primeira linha e tira as linhas "RD…", mantendo o resto. `false` se não há campo.
pub fn ajustar_vigor(mapa: &Mapa, valores: &mut Valores, vigor: Option<i64>, sem_rd: bool) -> bool {
    let vigor_campos: Vec<&Campo> = mapa.campos.iter().filter(|c| slug(&c.rotulo).split('-').any(|w| w == "vigor")).collect();
    let Some(campo) = vigor_campos.iter().find(|c| slug(&c.rotulo).ends_with("max")).or(vigor_campos.first()).copied() else { return false };
    if let (Tipo::Numero | Tipo::Bolinhas { .. }, Some(n)) = (&campo.tipo, vigor) {
        valores.insert(campo.id.clone(), Valor::Numero(n));
        return true;
    }
    let atual = match valores.get(&campo.id) { Some(Valor::Texto(t)) => t.clone(), Some(Valor::Numero(n)) => n.to_string(), _ => String::new() };
    let mut linhas: Vec<String> = atual.lines().map(str::to_string).collect();
    if sem_rd { linhas.retain(|l| !slug(l).starts_with("rd")); }
    if let Some(n) = vigor {
        match linhas.iter().position(|l| l.trim().chars().all(|c| c.is_ascii_digit()) && !l.trim().is_empty()) {
            Some(i) => linhas[i] = n.to_string(),
            None => linhas.insert(0, n.to_string()),
        }
    }
    valores.insert(campo.id.clone(), Valor::Texto(linhas.join("\n")));
    true
}

/// Pedido de ficha nova a partir de outra ("cria uma ficha pra esse monstro com os valores base da
/// Besta Ark"): verbo de criar e a palavra ficha. A ficha base é achada pelo nome, como nas edições.
pub fn pede_nova(message: &str) -> bool {
    let words: Vec<String> = slug(message).split('-').map(str::to_string).collect();
    let criar = ["cria", "crie", "criar", "faz", "faca", "fazer", "monta", "monte", "montar", "gera", "gere", "gerar", "nova", "novo", "duplica", "duplique", "copia", "copie"];
    words.iter().any(|w| w == "ficha" || w == "fichas") && words.iter().any(|w| criar.contains(&w.as_str()))
}

/// Nome da ficha nova dito no pedido: "chamada Caranguejo Vulcânico", "com o nome X", entre aspas
/// ou "ficha do Caranguejo Vulcânico a partir da…". `None` quando o pedido só aponta ("esse monstro").
pub fn nome_pedido(message: &str) -> Option<String> {
    let clean = |text: &str| -> Option<String> {
        let stops = [",", ".", ";", "!", "?", "\n", " a partir", " baseado", " baseada", " com ", " use ", " usando", " igual", " copiando", " no ", " na ", " nos ", " nas ", " do mesmo", " da mesma", " e "];
        let mut end = text.len();
        for stop in stops { if let Some(at) = text.find(stop) { end = end.min(at); } }
        let name = text[..end].trim().trim_matches(|c: char| c == '"' || c == '“' || c == '”' || c == '\'').trim();
        let lower = name.to_lowercase();
        let pointer = ["esse", "essa", "este", "esta", "ele", "ela", "isso", "o monstro", "a fera", "monstro", "fera", "criatura", "personagem", "imagem", "mim"];
        (name.chars().any(char::is_alphabetic) && name.split_whitespace().count() <= 6 && !pointer.iter().any(|p| lower == *p || lower.starts_with(&format!("{p} "))))
            .then(|| nome_proprio(name))
    };
    for (open, close) in [('“', '”'), ('"', '"')] {
        if let Some(start) = message.find(open) {
            let rest = &message[start + open.len_utf8()..];
            if let Some(end) = rest.find(close) { if let Some(name) = clean(&rest[..end]) { return Some(name); } }
        }
    }
    let lower = message.to_lowercase();
    for marker in ["chamado ", "chamada ", "com o nome ", "com nome ", "de nome ", "nomeado ", "nomeada ", "ficha do ", "ficha da ", "ficha de ", "ficha pro ", "ficha pra ", "ficha para o ", "ficha para a "] {
        for (at, _) in lower.match_indices(marker) {
            if let Some(name) = message.get(at + marker.len()..).and_then(clean) { return Some(name); }
        }
    }
    None
}

/// Nome a partir do arquivo de uma imagem anexada ("Volcanic Tidepool Crustacean Beast.png"):
/// `None` para nomes de câmera ou só números ("IMG_2034", "image").
pub fn nome_de_arquivo(file: &str) -> Option<String> {
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    let words: Vec<&str> = stem.split(|c: char| c == '_' || c == '-' || c.is_whitespace()).filter(|w| !w.is_empty()).collect();
    let letters = |w: &str| w.chars().filter(|c| c.is_alphabetic()).count();
    let junk = ["img", "image", "imagem", "screenshot", "captura", "foto", "photo", "pxl", "dsc", "whatsapp", "download"];
    let good: Vec<&str> = words.iter().copied().filter(|w| letters(w) >= 2 && letters(w) * 2 > w.chars().count()).collect();
    if good.is_empty() || good.iter().any(|w| junk.contains(&w.to_lowercase().as_str())) || good.len() * 2 < words.len() { return None; }
    Some(good.join(" "))
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
    if let Some(w) = fonte_do_campo(|f| f.largura(text)) { return w * size / 1000.0; }
    winansi(text).iter().map(|&b| if b >= 32 { WIDTHS[(b - 32) as usize] as f64 } else { 0.0 }).sum::<f64>() * size / 1000.0
}

/// Fonte da própria ficha (a do Canva, embutida no PDF só com as letras que ela usa): a Ayla
/// escreve com ela quando todas as letras do texto estão lá; senão, em Helvetica.
#[derive(Clone, Debug, Default)]
pub struct Fonte {
    /// Letra → glifo (CID, Identity-H).
    glifos: BTreeMap<char, u16>,
    larguras: BTreeMap<u16, f64>,
    padrao: f64,
}

impl Fonte {
    fn cobre(&self, text: &str) -> bool { text.chars().all(|c| self.glifos.contains_key(&c)) }
    /// Largura em milésimos do tamanho, se todas as letras existirem.
    fn largura(&self, text: &str) -> Option<f64> {
        self.cobre(text).then(|| text.chars().map(|c| self.larguras.get(&self.glifos[&c]).copied().unwrap_or(self.padrao)).sum())
    }
    fn hex(&self, text: &str) -> String {
        format!("<{}>", text.chars().map(|c| format!("{:04X}", self.glifos[&c])).collect::<String>())
    }
    /// Lê o mapa de letras (ToUnicode) e as larguras (/W) de uma fonte Type0 Identity-H.
    pub fn ler(doc: &lopdf::Document, fonte: &lopdf::Dictionary) -> Option<Self> {
        use lopdf::Object;
        let deref = |o: &Object| match o { Object::Reference(r) => doc.get_object(*r).ok().cloned(), other => Some(other.clone()) };
        if fonte.get(b"Subtype").ok()?.as_name().ok()? != b"Type0" || fonte.get(b"Encoding").ok().and_then(|e| e.as_name().ok()) != Some(b"Identity-H".as_slice()) { return None; }
        let cmap = match deref(fonte.get(b"ToUnicode").ok()?)? { Object::Stream(s) => s.decompressed_content().ok().unwrap_or(s.content.clone()), _ => return None };
        let glifos = to_unicode(&String::from_utf8_lossy(&cmap));
        if glifos.is_empty() { return None; }
        let descendente = match deref(fonte.get(b"DescendantFonts").ok()?)? { Object::Array(a) => deref(a.first()?)?, _ => return None };
        let descendente = descendente.as_dict().ok()?;
        let padrao = descendente.get(b"DW").ok().and_then(|o| o.as_float().ok().map(f64::from).or_else(|| o.as_i64().ok().map(|v| v as f64))).unwrap_or(1000.0);
        let mut larguras = BTreeMap::new();
        if let Some(Object::Array(w)) = descendente.get(b"W").ok().and_then(deref) {
            let num = |o: &Object| deref(o).and_then(|o| o.as_float().ok().map(f64::from).or_else(|| o.as_i64().ok().map(|v| v as f64)));
            let mut i = 0;
            while i < w.len() {
                let Some(first) = num(&w[i]) else { break };
                match w.get(i + 1).and_then(deref) {
                    Some(Object::Array(list)) => { for (k, v) in list.iter().enumerate() { if let Some(v) = num(v) { larguras.insert(first as u16 + k as u16, v); } } i += 2; }
                    Some(_) => { let (Some(last), Some(v)) = (num(&w[i + 1]), w.get(i + 2).and_then(num)) else { break }; for c in first as u16..=last as u16 { larguras.insert(c, v); } i += 3; }
                    None => break,
                }
            }
        }
        Some(Fonte { glifos, larguras, padrao })
    }
}

/// League Spartan Bold completa (OFL), a fonte das fichas do Canva: escreve qualquer letra nelas.
const LEAGUE_SPARTAN: &[u8] = include_bytes!("../../fonts/LeagueSpartan-Bold.ttf");

/// Métricas de uma TrueType para o descritor do PDF (em milésimos).
struct Metricas { ascent: f64, descent: f64, bbox: [f64; 4] }

/// Lê de uma TrueType a tabela de letras (Latim, U+0020 a U+02FF) e as larguras, em milésimos.
fn fonte_ttf(ttf: &[u8]) -> Option<(Fonte, Metricas)> {
    let u16_at = |o: usize| ttf.get(o..o + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let i16_at = |o: usize| u16_at(o).map(|v| v as i16 as f64);
    let u32_at = |o: usize| ttf.get(o..o + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let tabela = |tag: &[u8; 4]| (0..u16_at(4)? as usize).find_map(|i| { let r = 12 + i * 16; (ttf.get(r..r + 4)? == tag).then(|| u32_at(r + 8)).flatten() });
    let (head, hhea, hmtx, cmap) = (tabela(b"head")?, tabela(b"hhea")?, tabela(b"hmtx")?, tabela(b"cmap")?);
    let upem = u16_at(head + 18)? as f64;
    let escala = |v: f64| v * 1000.0 / upem;
    let metricas = Metricas { ascent: escala(i16_at(hhea + 4)?), descent: escala(i16_at(hhea + 6)?), bbox: [escala(i16_at(head + 36)?), escala(i16_at(head + 38)?), escala(i16_at(head + 40)?), escala(i16_at(head + 42)?)] };
    let metricas_h = u16_at(hhea + 34)? as usize;
    let avanco = |g: u16| { let i = (g as usize).min(metricas_h.saturating_sub(1)); u16_at(hmtx + i * 4).map(|w| escala(w as f64)) };
    // Subtabela Unicode BMP (formato 4).
    let sub = (0..u16_at(cmap + 2)? as usize).find_map(|i| {
        let r = cmap + 4 + i * 8;
        let (plataforma, codificacao) = (u16_at(r)?, u16_at(r + 2)?);
        let off = cmap + u32_at(r + 4)?;
        ((plataforma == 3 && codificacao == 1) || plataforma == 0).then_some(off).filter(|o| u16_at(*o) == Some(4))
    })?;
    let segs = u16_at(sub + 6)? as usize / 2;
    let (fins, inicios, deltas, faixas) = (sub + 14, sub + 16 + segs * 2, sub + 16 + segs * 4, sub + 16 + segs * 6);
    let glifo = |c: u32| -> Option<u16> {
        let i = (0..segs).find(|&i| u16_at(fins + i * 2).is_some_and(|f| f as u32 >= c))?;
        let inicio = u16_at(inicios + i * 2)? as u32;
        if inicio > c { return None; }
        let delta = u16_at(deltas + i * 2)?;
        let faixa = u16_at(faixas + i * 2)? as usize;
        let g = if faixa == 0 { (c as u16).wrapping_add(delta) } else {
            let g = u16_at(faixas + i * 2 + faixa + (c - inicio) as usize * 2)?;
            if g == 0 { return None; }
            g.wrapping_add(delta)
        };
        (g != 0).then_some(g)
    };
    let mut fonte = Fonte { padrao: avanco(0)?, ..Default::default() };
    for c in 0x20u32..0x300 {
        let (Some(ch), Some(g)) = (char::from_u32(c), glifo(c)) else { continue };
        fonte.glifos.insert(ch, g);
        if let Some(w) = avanco(g) { fonte.larguras.insert(g, w); }
    }
    (!fonte.glifos.is_empty()).then_some((fonte, metricas))
}

/// Embute a League Spartan no PDF (Type0 / CIDFontType2, Identity-H, com ToUnicode para o texto
/// continuar copiável) e devolve o objeto da fonte e o mapa para escrever com ela.
fn embutir_league(doc: &mut lopdf::Document) -> Option<(lopdf::ObjectId, Fonte)> {
    use lopdf::{Dictionary, Object, Stream};
    let (fonte, m) = fonte_ttf(LEAGUE_SPARTAN)?;
    let nome = || Object::Name(b"LeagueSpartan-Bold".to_vec());
    let mut arquivo = Stream::new(Dictionary::from_iter([("Length1", Object::Integer(LEAGUE_SPARTAN.len() as i64))]), LEAGUE_SPARTAN.to_vec());
    let _ = arquivo.compress();
    let arquivo = doc.add_object(Object::Stream(arquivo));
    let descritor = doc.add_object(Object::Dictionary(Dictionary::from_iter([
        ("Type", Object::Name(b"FontDescriptor".to_vec())), ("FontName", nome()), ("Flags", Object::Integer(32)),
        ("FontBBox", Object::Array(m.bbox.iter().map(|v| Object::Integer(*v as i64)).collect())), ("ItalicAngle", Object::Integer(0)),
        ("Ascent", Object::Integer(m.ascent as i64)), ("Descent", Object::Integer(m.descent as i64)), ("CapHeight", Object::Integer(700)),
        ("StemV", Object::Integer(140)), ("FontFile2", Object::Reference(arquivo)),
    ])));
    let mut larguras: Vec<Object> = Vec::new();
    for (g, w) in &fonte.larguras { larguras.push(Object::Integer(*g as i64)); larguras.push(Object::Array(vec![Object::Integer(w.round() as i64)])); }
    let cid = doc.add_object(Object::Dictionary(Dictionary::from_iter([
        ("Type", Object::Name(b"Font".to_vec())), ("Subtype", Object::Name(b"CIDFontType2".to_vec())), ("BaseFont", nome()),
        ("CIDSystemInfo", Object::Dictionary(Dictionary::from_iter([("Registry", Object::string_literal("Adobe")), ("Ordering", Object::string_literal("Identity")), ("Supplement", Object::Integer(0))]))),
        ("FontDescriptor", Object::Reference(descritor)), ("DW", Object::Integer(fonte.padrao.round() as i64)), ("W", Object::Array(larguras)),
        ("CIDToGIDMap", Object::Name(b"Identity".to_vec())),
    ])));
    let pares: Vec<String> = fonte.glifos.iter().map(|(c, g)| format!("<{g:04X}> <{:04X}>", *c as u32)).collect();
    let mut cmap = String::from("/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n");
    for bloco in pares.chunks(100) { cmap.push_str(&format!("{} beginbfchar\n{}\nendbfchar\n", bloco.len(), bloco.join("\n"))); }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    let mut unicode = Stream::new(Dictionary::new(), cmap.into_bytes());
    let _ = unicode.compress();
    let unicode = doc.add_object(Object::Stream(unicode));
    let tipo0 = doc.add_object(Object::Dictionary(Dictionary::from_iter([
        ("Type", Object::Name(b"Font".to_vec())), ("Subtype", Object::Name(b"Type0".to_vec())), ("BaseFont", nome()),
        ("Encoding", Object::Name(b"Identity-H".to_vec())), ("DescendantFonts", Object::Array(vec![Object::Reference(cid)])), ("ToUnicode", Object::Reference(unicode)),
    ])));
    Some((tipo0, fonte))
}

/// CMap ToUnicode → letra para glifo (só os glifos de uma letra; o espaço também).
fn to_unicode(cmap: &str) -> BTreeMap<char, u16> {
    let hex = |t: &str| u32::from_str_radix(t.trim_matches(|c| c == '<' || c == '>'), 16).ok();
    let letra = |t: &str| { let t = t.trim_matches(|c| c == '<' || c == '>'); (t.len() == 4).then(|| u32::from_str_radix(t, 16).ok().and_then(char::from_u32)).flatten() };
    let mut out = BTreeMap::new();
    let mut put = |c: char, g: u32| { if let Ok(g) = u16::try_from(g) { out.entry(c).or_insert(g); } };
    let mut modo = "";
    for line in cmap.lines() {
        let line = line.trim();
        if line.ends_with("beginbfchar") { modo = "char"; continue; }
        if line.ends_with("beginbfrange") { modo = "range"; continue; }
        if line.starts_with("endbf") { modo = ""; continue; }
        let parts: Vec<&str> = line.split_whitespace().collect();
        match (modo, parts.as_slice()) {
            ("char", [g, u]) => if let (Some(g), Some(c)) = (hex(g), letra(u)) { put(c, g) },
            ("range", [a, b, u]) if !u.starts_with('[') => if let (Some(a), Some(b), Some(start)) = (hex(a), hex(b), hex(u)) {
                for (k, g) in (a..=b).enumerate() { if let Some(c) = char::from_u32(start + k as u32) { put(c, g) } }
            },
            ("range", [a, _b, rest @ ..]) => if let Some(a) = hex(a) {
                for (k, u) in rest.iter().map(|t| t.trim_matches(|c| c == '[' || c == ']')).filter(|t| !t.is_empty()).enumerate() { if let Some(c) = letra(u) { put(c, a + k as u32) } }
            },
            _ => {}
        }
    }
    out
}

thread_local! {
    /// Fonte da página que está sendo desenhada (medir e escrever usam a mesma).
    static FONTE: std::cell::RefCell<Option<Fonte>> = const { std::cell::RefCell::new(None) };
    /// O campo em desenho tem letra fora da fonte da ficha: ele todo vai em Helvetica (sem misturar).
    static SO_HELVETICA: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Fonte da ficha, se ela vale para o campo atual.
fn fonte_do_campo<T>(f: impl FnOnce(&Fonte) -> Option<T>) -> Option<T> {
    if SO_HELVETICA.with(|s| s.get()) { return None; }
    FONTE.with(|fonte| fonte.borrow().as_ref().and_then(f))
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
        let propria = fonte_do_campo(|f| f.cobre(text).then(|| f.hex(text)));
        let (fonte, codigo) = match propria { Some(hex) => ("AylaE", hex), None => ("AylaF", pdf_string(text)) };
        self.ops.push_str(&format!("BT 0.08 0.08 0.1 rg /{fonte} {size:.2} Tf {:.2} {:.2} Td {codigo} Tj ET\n", self.x0 + x, self.y(baseline_top)));
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
fn desenhar(tela: &mut Tela, campo: &Campo, valor: Option<&Valor>, dica: Option<Dica>, linhas: &[(f64, [u8; 3])]) {
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
            // O texto original estava no meio da caixa (o "120" do Vigor): o novo também.
            let meio = dica.and_then(|d| d.faixa).is_some_and(|(a, b)| ((a + b) / 2.0 - centro(inner).0).abs() < largura(inner) * 0.08 && b - a < largura(inner) * 0.85);
            for (i, line) in quebrar(t, size, largura(inner)).iter().enumerate() {
                let x = if meio { inner[0] + (largura(inner) - medir(line, size)).max(0.0) / 2.0 } else { inner[0] };
                tela.texto(line, x, inner[1] + size * (1.2 * i as f64 + 0.95), size);
            }
        }
        (Tipo::Tracos, Some(Valor::Tracos(list))) => {
            // Título maior e centralizado; descrição menor, centralizada embaixo dele.
            let um = |t: &Traco, s: f64| {
                let title = quebrar(&t.titulo, s * 1.25, largura(inner)).len() as f64 * s * 1.25 * 1.15;
                let desc = if t.descricao.is_empty() { 0.0 } else { quebrar(&t.descricao, s, largura(inner)).len() as f64 * s * 1.2 };
                title + desc
            };
            let escrever = |tela: &mut Tela, t: &Traco, size: f64, mut top: f64| {
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
                top
            };
            // O tamanho da descrição original (o título é 1,25 vez maior), sem encolher à toa.
            let maximo = dica.map_or(8.5, |d| d.tamanho.min(14.0));
            // Linhas cinza que separam os traços na ficha: cada traço no seu espaço; o que não cabe
            // num só ocupa os seguintes (a linha do meio é apagada, como no original).
            let cortes: Vec<(f64, [u8; 3])> = linhas.iter().copied().filter(|(y, _)| *y > inner[1] + 6.0 && *y < inner[3] - 6.0).collect();
            let mut espacos: Vec<(f64, f64)> = Vec::new();
            let mut topo = inner[1];
            for (y, _) in &cortes { espacos.push((topo, *y)); topo = *y; }
            espacos.push((topo, inner[3]));
            let folga = 5.0;
            let distribuir = |size: f64| -> Option<Vec<(usize, usize)>> {
                let mut livre = 0;
                let mut out = Vec::new();
                for t in list {
                    let precisa = um(t, size) + folga * 2.0;
                    let mut fim = livre;
                    while fim < espacos.len() && espacos[fim].1 - espacos.get(livre)?.0 < precisa { fim += 1; }
                    if fim >= espacos.len() { return None; }
                    out.push((livre, fim));
                    livre = fim + 1;
                }
                Some(out)
            };
            let mut size = maximo;
            let mut lugar = None;
            if !cortes.is_empty() {
                while size >= 4.0 { if let Some(l) = distribuir(size) { lugar = Some(l); break; } size -= 0.25; }
            }
            if let Some(lugar) = lugar {
                for (t, (a, b)) in list.iter().zip(lugar) {
                    for (y, cor) in &cortes[a..b] { tela.retangulo([campo.caixa[0] + 1.0, y - 1.6, campo.caixa[2] - 1.0, y + 1.6], *cor); }
                    let (topo, fundo) = (espacos[a].0, espacos[b].1);
                    escrever(tela, t, size, (topo + (fundo - topo - um(t, size)) / 2.0).max(topo + folga));
                }
            } else {
                let size = caber(inner, maximo, |s| list.iter().map(|t| um(t, s) + s * 1.1).sum::<f64>());
                let mut top = inner[1];
                for t in list { top = escrever(tela, t, size, top) + size * 1.1; }
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
fn camada(campos: &[(&Campo, Option<&Valor>)], todos: &[&Campo], pagina: &Pagina, original: &Pagina, imagem: &Imagem, altura_pdf: f64, x0: f64, fonte: Option<Fonte>) -> String {
    FONTE.with(|f| *f.borrow_mut() = fonte);
    let ops = camada_com_fonte(campos, todos, pagina, original, imagem, altura_pdf, x0);
    FONTE.with(|f| *f.borrow_mut() = None);
    ops
}

fn camada_com_fonte(campos: &[(&Campo, Option<&Valor>)], todos: &[&Campo], pagina: &Pagina, original: &Pagina, imagem: &Imagem, altura_pdf: f64, x0: f64) -> String {
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
        // Linhas cinza dos Traços, com a cor do fundo em volta de cada uma (para apagar a do meio de
        // um traço que ocupa dois espaços).
        let linhas: Vec<(f64, [u8; 3])> = if campo.tipo == Tipo::Tracos {
            imagem.linhas_pontilhadas(campo.caixa).into_iter().map(|y| (y, imagem.fundo([campo.caixa[0] + 4.0, y - 1.2, campo.caixa[2] - 4.0, y + 1.2]))).collect()
        } else { Vec::new() };
        let textos: String = match valor {
            Some(Valor::Texto(t)) => t.chars().filter(|c| *c != '\n').collect(),
            Some(Valor::Tracos(l)) => l.iter().map(|t| format!("{}{}", t.titulo, t.descricao)).collect(),
            Some(Valor::Numero(n)) => n.to_string(),
            None => String::new(),
        };
        let cobre = FONTE.with(|f| f.borrow().as_ref().is_none_or(|f| f.cobre(&textos)));
        SO_HELVETICA.with(|s| s.set(!cobre));
        desenhar(&mut tela, campo, *valor, d, &linhas);
        SO_HELVETICA.with(|s| s.set(false));
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
    let mut completa: Option<(lopdf::ObjectId, Fonte)> = None;
    for (number, campos) in by_page {
        let (page, mbox) = boxes[&number];
        let texto = palavras(&stripped, number)?;
        let pixels = imagem(original, number)?;
        let antes = palavras(original, number)?;
        // A fonte da ficha nesta página (a Type0 com mais letras), usada pelo texto novo.
        let propria: Option<(lopdf::ObjectId, Fonte)> = {
            let dict_of = |o: Option<&Object>| match o { Some(Object::Dictionary(d)) => d.clone(), Some(Object::Reference(r)) => doc.get_dictionary(*r).cloned().unwrap_or_default(), _ => Dictionary::new() };
            let resources = recursos(&doc, page)?;
            // As fontes da página e as dos desenhos (XObjects) dela, um nível abaixo (o Canva usa).
            let mut ids: Vec<lopdf::ObjectId> = dict_of(resources.get(b"Font").ok()).iter().filter_map(|(_, o)| o.as_reference().ok()).collect();
            for (_, x) in dict_of(resources.get(b"XObject").ok()).iter() {
                let Ok(id) = x.as_reference() else { continue };
                if let Ok(Object::Stream(form)) = doc.get_object(id) {
                    let inner = dict_of(form.dict.get(b"Resources").ok());
                    ids.extend(dict_of(inner.get(b"Font").ok()).iter().filter_map(|(_, o)| o.as_reference().ok()));
                }
            }
            let achada = ids.into_iter().filter_map(|id| Some((id, Fonte::ler(&doc, doc.get_dictionary(id).ok()?)?))).max_by_key(|(_, f)| f.glifos.len());
            // Ficha em League Spartan (a do Canva): a fonte completa, embutida uma vez no PDF.
            let league = achada.as_ref().and_then(|(id, _)| doc.get_dictionary(*id).ok()?.get(b"BaseFont").ok()?.as_name().ok().map(|n| String::from_utf8_lossy(n).contains("LeagueSpartan")));
            if league == Some(true) {
                if completa.is_none() { completa = embutir_league(&mut doc); }
                completa.clone().or(achada)
            } else { achada }
        };
        let ops = camada(&campos, &todos[&number], &texto, &antes, &pixels, mbox[3], mbox[0], propria.as_ref().map(|(_, f)| f.clone()));
        let mut form = Dictionary::new();
        form.set("Type", Object::Name(b"XObject".to_vec()));
        form.set("Subtype", Object::Name(b"Form".to_vec()));
        form.set("BBox", Object::Array(mbox.iter().map(|v| Object::Real(*v as f32)).collect()));
        let mut fonts = Dictionary::new();
        fonts.set("AylaF", Object::Reference(font_id));
        if let Some((id, _)) = &propria { fonts.set("AylaE", Object::Reference(*id)); }
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

    /// Ficha real de uma página só, já preenchida (`AYLA_FICHA_UNICA`): campos e nome lidos.
    #[test]
    #[ignore = "usa uma ficha real indicada por variável de ambiente"]
    fn ficha_unica_real() {
        let pdf = PathBuf::from(std::env::var("AYLA_FICHA_UNICA").expect("AYLA_FICHA_UNICA"));
        let pagina = palavras(&pdf, 1).unwrap();
        let campos = propor(&pagina, &imagem(&pdf, 1).unwrap(), 0);
        for c in &campos { eprintln!("{:28} {:?} {:?}", c.rotulo, c.tipo, c.caixa); }
        if std::env::var("AYLA_FICHA_DEPURAR").is_ok() {
            let img = imagem(&pdf, 1).unwrap();
            for l in rotulos(&pagina.palavras).iter().filter(|l| ["VIGOR", "ATUAL", "MAX", "DURABILIDADE", "ATUAL:", "MAX:", "CONDIÇÕES"].iter().any(|t| l.texto == t.trim_end_matches(':'))) {
                let (_, cy) = centro(l.caixa);
                let own = img.celula(l.caixa[2] + 2.0, cy).or_else(|| img.celula(l.caixa[2] + 4.0, cy));
                let below = [2.5, 5.0, 8.0, 12.0].iter().find_map(|d| img.celula(centro(l.caixa).0, l.caixa[3] + d));
                eprintln!("rótulo {:?} {:?} own={:?} below={:?}", l.texto, l.caixa.map(|v| v.round()), own.map(|c| c.map(|v| v.round())), below.map(|c| c.map(|v| v.round())));
            }
        }
        let mapa = Mapa { nome: "Real".into(), paginas: 1, campos };
        eprintln!("nome: {:?}", nome_na_pagina(&mapa, &pagina));
        for (k, v) in ler(&mapa, &pdf, 1).unwrap() { eprintln!("valor {k} = {v:?}"); }
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
    fn league_spartan_completa_tem_as_letras_da_lingua() {
        let (fonte, m) = fonte_ttf(LEAGUE_SPARTAN).expect("TrueType lida");
        assert!(fonte.cobre("ÁÂÃÀÇÉÊÍÓÔÕÚÜ áâãàçéêíóôõúü +5 (MÁXIMO) 1D6"), "letras do português");
        let a = fonte.largura("A").unwrap();
        assert!(a > 400.0 && a < 900.0, "largura do A em milésimos: {a}");
        assert!(m.ascent > 600.0 && m.descent < 0.0);
    }

    #[test]
    fn fonte_da_ficha_mapa_de_letras() {
        let cmap = "/CIDInit /ProcSet findresource begin\n2 beginbfchar\n<0003> <0020>\n<0010> <00C7>\nendbfchar\n2 beginbfrange\n<0024> <0026> <0041>\n<0030> <0031> [<0035> <0036>]\nendbfrange\nend";
        let glifos = to_unicode(cmap);
        assert_eq!((glifos[&' '], glifos[&'Ç'], glifos[&'A'], glifos[&'C'], glifos[&'5'], glifos[&'6']), (3, 0x10, 0x24, 0x26, 0x30, 0x31));
        let fonte = Fonte { glifos, larguras: BTreeMap::from([(0x24, 600.0)]), padrao: 500.0 };
        assert_eq!(fonte.largura("A C"), Some(1600.0));
        assert_eq!(fonte.largura("AZ"), None, "letra fora da fonte: a linha vai em Helvetica");
        assert_eq!(fonte.hex("AÇ"), "<00240010>");
    }

    #[test]
    fn pedido_de_ficha_nova_e_nome() {
        let pedido = "cria pra mim no wilderfeast uma ficha pra esse monstro, use os valores base da Besta Ark";
        assert!(pede_nova(pedido));
        assert!(pede_mudanca("altera o nome dele pra Casco de Cinzas, coloca 50 de vida e remove o RD"));
        assert!(!pede_mudanca("o que você acha desse monstro?"));
        let pedido = "altera o nome dele pra Casco de Cinzas, coloca 50 de vida e remove o RD. remova os traços exceto pelo primeiro";
        assert_eq!(pedido_vigor(pedido), Some(50));
        assert_eq!(regra_tracos(pedido), Some(1));
        assert_eq!(pede_tracos_novos("remova os traços exceto pelo primeiro, e coloque no lugar habilidades que voce acha que faça juz a esse monstro. podem ser 2 no total"), Some(2));
        assert_eq!(pede_tracos_novos("cria três traços de gelo"), Some(3));
        assert_eq!(pede_tracos_novos("coloca 50 de vida"), None);
        assert_eq!(regra_tracos("deixa só os dois primeiros traços e cria mais um"), Some(2));
        assert_eq!(regra_tracos("substitua os traços por habilidades de fogo"), Some(0));
        assert_eq!(regra_tracos("adiciona um traço de voo"), None);
        assert_eq!(regra_tracos("troca o segundo traço por um traço novo de fogo"), None, "um só: quem diz qual é o modelo");
        assert_eq!(regra_tracos("tira todos os traços e cria três de gelo"), Some(0));
        assert_eq!(regra_tracos("remove os traços de voo"), None);
        let mut campos = vec![Campo { id: "tracos".into(), rotulo: "Traços".into(), pagina: 0, caixa: [0.0; 4], tipo: Tipo::Bloco }];
        tipos_conhecidos(&mut campos);
        assert_eq!(campos[0].tipo, Tipo::Tracos);
        assert_eq!(pedido_vigor("muda o vigor para 80"), Some(80));
        assert_eq!(pedido_vigor("ele tem muita vida"), None);
        assert!(pede_sem_rd(pedido) && !pede_sem_rd("coloca RD 3"));
        let mapa = Mapa { nome: "Fera".into(), paginas: 1, campos: vec![
            Campo { id: "vigor-atual".into(), rotulo: "Vigor · Atual".into(), pagina: 0, caixa: [0.0; 4], tipo: Tipo::Bloco },
            Campo { id: "vigor-max".into(), rotulo: "Vigor · Max".into(), pagina: 0, caixa: [0.0; 4], tipo: Tipo::Bloco }] };
        let mut valores = Valores::new();
        valores.insert("vigor-max".into(), Valor::Texto("120\nRDA 2".into()));
        assert!(ajustar_vigor(&mapa, &mut valores, Some(50), true));
        assert_eq!(valores["vigor-max"], Valor::Texto("50".into()));
        assert!(!valores.contains_key("vigor-atual"));
        assert!(!pede_nova("adiciona um ponto em Golpear na ficha do Bartholomew"));
        assert_eq!(nome_pedido(pedido), None, "\"esse monstro\" não é nome");
        assert_eq!(nome_pedido("cria a ficha do Caranguejo Vulcânico a partir da Besta Ark").as_deref(), Some("Caranguejo Vulcânico"));
        assert_eq!(nome_pedido("faz uma ficha chamada caranguejo de lava, com os valores da Besta Ark").as_deref(), Some("Caranguejo de Lava"));
        assert_eq!(nome_pedido("cria uma ficha “Rei das Marés” igual à do Basilisco").as_deref(), Some("Rei das Marés"));
        assert_eq!(nome_de_arquivo("Volcanic Tidepool Crustacean Beast.png").as_deref(), Some("Volcanic Tidepool Crustacean Beast"));
        assert_eq!(nome_de_arquivo("IMG_2034.jpg"), None);
        assert_eq!(nome_de_arquivo("20261126_143000.png"), None);
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
        // Ficha de uma página só (sem modelo em branco): o nome preenchido é valor de "Nome", não rótulo.
        let da_cheia = propor(&palavras(&cheia, 1).unwrap(), &imagem(&cheia, 1).unwrap(), 0);
        assert!(da_cheia.iter().all(|c| c.id != "bartholomew"), "{da_cheia:?}");
        let mapa_cheia = Mapa { nome: "Cheia".into(), paginas: 1, campos: da_cheia };
        assert_eq!(campo_nome(&mapa_cheia).map(|c| c.id.as_str()), Some("nome"));
        assert_eq!(nome_na_pagina(&mapa_cheia, &palavras(&cheia, 1).unwrap()).as_deref(), Some("Bartholomew"));
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
