//! Hand: a mão da Ayla no computador. Capturas de tela, busca de arquivos, organização de pastas
//! (plano com prévia, Aplicar e desfazer), programas e comandos. Tudo determinístico e local: a
//! página do Hand e a conversa usam estes mesmos serviços, e nada daqui vai para a nuvem.
//!
//! Segurança: só lê a pasta pessoal (sem pastas ocultas nem os dados da Ayla); só muda arquivos
//! dentro das pastas liberadas pela pessoa; nunca sobrescreve (nome repetido vira "nome 2");
//! apagar manda para a lixeira do sistema.
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// Pasta do usuário pelo `~/.config/user-dirs.dirs` (DOWNLOAD, DOCUMENTS, PICTURES, DESKTOP…).
pub fn user_dir(key: &str) -> Option<PathBuf> {
    let home = home();
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    let text = fs::read_to_string(config.join("user-dirs.dirs")).ok()?;
    let wanted = format!("XDG_{key}_DIR=");
    let line = text.lines().map(str::trim).find(|l| l.starts_with(&wanted))?;
    let value = line[wanted.len()..].trim().trim_matches('"');
    let path = match value.strip_prefix("$HOME") { Some(rest) => home.join(rest.trim_start_matches('/')), None => PathBuf::from(value) };
    (path != home).then_some(path)
}

/// Pastas liberadas por padrão: Downloads, Documentos, Imagens e Área de trabalho (as que existem).
pub fn default_folders() -> Vec<PathBuf> {
    let home = home();
    [("DOWNLOAD", "Downloads"), ("DOCUMENTS", "Documents"), ("PICTURES", "Pictures"), ("DESKTOP", "Desktop")].iter()
        .map(|(key, fallback)| user_dir(key).unwrap_or_else(|| home.join(fallback)))
        .filter(|p| p.is_dir())
        .collect()
}

/// Onde as capturas ficam: Imagens/Capturas de tela.
pub fn screenshots_dir() -> PathBuf {
    user_dir("PICTURES").unwrap_or_else(|| home().join("Pictures")).join("Capturas de tela")
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn modified(meta: &fs::Metadata) -> i64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn hidden(path: &Path) -> bool {
    path.components().any(|c| matches!(c, Component::Normal(n) if n.to_string_lossy().starts_with('.')))
}

/// Caminho absoluto, sem `..`, dentro de uma das pastas (e não oculto abaixo dela).
pub fn inside(path: &Path, folders: &[PathBuf]) -> bool {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) { return false; }
    folders.iter().any(|f| path.strip_prefix(f).is_ok_and(|rest| !hidden(rest)))
}

// ---------------------------------------------------------------------------------------------
// Capturas de tela

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Captura { Tela, Regiao, Janela }

fn capture_name(dir: &Path) -> PathBuf {
    let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M-%S");
    free_name(&dir.join(format!("Captura {stamp}.png")))
}

/// Janelas visíveis (nos workspaces à mostra), como retângulos para o `slurp -r` escolher.
fn window_boxes() -> Vec<String> {
    let json = |what: &str| Command::new("hyprctl").args([what, "-j"]).output().ok()
        .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok());
    let shown: HashSet<i64> = json("monitors").and_then(|m| m.as_array().cloned()).unwrap_or_default().iter()
        .filter_map(|m| m["activeWorkspace"]["id"].as_i64()).collect();
    json("clients").and_then(|c| c.as_array().cloned()).unwrap_or_default().iter()
        .filter(|c| c["mapped"].as_bool() != Some(false) && c["hidden"].as_bool() != Some(true))
        .filter(|c| c["workspace"]["id"].as_i64().is_some_and(|id| shown.contains(&id)))
        .filter_map(|c| {
            let (x, y) = (c["at"][0].as_i64()?, c["at"][1].as_i64()?);
            let (w, h) = (c["size"][0].as_i64()?, c["size"][1].as_i64()?);
            (w > 0 && h > 0).then(|| format!("{x},{y} {w}x{h} {}", c["title"].as_str().unwrap_or("").replace('\n', " ")))
        })
        .collect()
}

/// Escolha da pessoa com o `slurp` (região livre ou uma das janelas). `None` se ela cancelar.
fn pick(boxes: Option<&[String]>) -> Result<Option<String>, String> {
    use std::io::Write;
    let mut command = Command::new("slurp");
    if boxes.is_some() { command.arg("-r"); }
    let mut child = command.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null())
        .spawn().map_err(|_| "Para escolher uma região ou janela, instale o slurp")?;
    if let (Some(boxes), Some(mut stdin)) = (boxes, child.stdin.take()) { let _ = stdin.write_all(boxes.join("\n").as_bytes()); }
    let out = child.wait_with_output().map_err(|_| "O slurp parou")?;
    let region = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok((out.status.success() && !region.is_empty()).then_some(region))
}

/// Tira a captura e a salva em `dir`. `None` quando a pessoa cancela a escolha.
pub fn capture(kind: Captura, dir: &Path, delay: u64) -> Result<Option<PathBuf>, String> {
    if Command::new("grim").arg("-h").output().is_err() { return Err("Para capturar a tela, instale o grim".into()); }
    let region = match kind {
        Captura::Tela => None,
        Captura::Regiao => match pick(None)? { Some(r) => Some(r), None => return Ok(None) },
        Captura::Janela => {
            let boxes = window_boxes();
            if boxes.is_empty() { return Err("Nenhuma janela visível para capturar".into()); }
            match pick(Some(&boxes))? { Some(r) => Some(r), None => return Ok(None) }
        }
    };
    if delay > 0 { std::thread::sleep(Duration::from_secs(delay.min(10))); }
    fs::create_dir_all(dir).map_err(|_| "Não foi possível criar a pasta das capturas")?;
    let path = capture_name(dir);
    let mut grim = Command::new("grim");
    if let Some(region) = &region { grim.args(["-g", region]); }
    let ok = grim.args(["-t", "png"]).arg(&path).status().map_err(|_| "O grim parou")?.success();
    if !ok || !path.is_file() { return Err("A captura falhou".into()); }
    Ok(Some(path))
}

// ---------------------------------------------------------------------------------------------
// Busca

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Tipo { Pasta, Imagem, Video, Audio, Pdf, Documento, Compactado, Instalador }

pub fn tipo_de(path: &Path) -> Option<Tipo> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    Some(match ext.as_str() {
        "png" | "jpg" | "jpeg" | "jfif" | "gif" | "webp" | "svg" | "heic" | "heif" | "avif" | "bmp" | "tif" | "tiff" => Tipo::Imagem,
        "mp4" | "m4v" | "mkv" | "webm" | "mov" | "avi" | "ogv" | "wmv" => Tipo::Video,
        "mp3" | "flac" | "ogg" | "oga" | "opus" | "wav" | "m4a" | "aac" => Tipo::Audio,
        "pdf" => Tipo::Pdf,
        "doc" | "docx" | "odt" | "txt" | "md" | "rtf" | "xls" | "xlsx" | "ods" | "csv" | "ppt" | "pptx" | "odp" | "epub" => Tipo::Documento,
        "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "xz" | "bz2" | "zst" => Tipo::Compactado,
        "rpm" | "deb" | "appimage" | "flatpakref" | "iso" | "exe" | "msi" => Tipo::Instalador,
        _ => return None,
    })
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Achado {
    pub path: String,
    pub name: String,
    pub folder: bool,
    pub bytes: u64,
    pub modified: i64,
}

/// Minúsculas e sem acentos, para comparar nomes ("Relatório" acha "relatorio").
pub fn fold(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).map(|c| match c {
        'á' | 'à' | 'â' | 'ã' | 'ä' => 'a', 'é' | 'è' | 'ê' | 'ë' => 'e', 'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o', 'ú' | 'ù' | 'û' | 'ü' => 'u', 'ç' => 'c', 'ñ' => 'n', c => c,
    }).collect()
}

/// Pastas pesadas que nunca têm o que a pessoa procura pelo nome.
const SKIP: &[&str] = &["node_modules", "target", "__pycache__", "venv", "dist", "build"];

pub struct Busca<'a> {
    pub query: &'a str,
    /// Tipos aceitos (vazio: qualquer um).
    pub tipos: &'a [Tipo],
    /// Também dentro de pastas e arquivos ocultos (os que começam com ".").
    pub ocultos: bool,
    /// Só alterados nos últimos N dias.
    pub dias: Option<u32>,
    pub limit: usize,
}

/// Pastas do sistema que não são arquivos de verdade (na busca em todo o computador).
pub const VIRTUAIS: &[&str] = &["/proc", "/sys", "/dev", "/run"];

/// Procura pelo nome (todas as palavras, sem diferenciar acentos) dentro de `root`, fora de
/// `exclude` (e de pastas ocultas, salvo `ocultos`). Usa o índice do `plocate` quando existe e uma varredura limitada das
/// pastas do dia a dia (para achar o que é mais novo que o índice).
pub fn search(busca: &Busca, root: &Path, exclude: &[PathBuf], fresh: &[PathBuf]) -> Vec<Achado> {
    let words: Vec<String> = fold(busca.query).split_whitespace().map(str::to_string).collect();
    if words.is_empty() && busca.tipos.is_empty() && busca.dias.is_none() { return Vec::new(); }
    let since = busca.dias.map(|d| now_secs() - i64::from(d) * 86_400);
    let accept = |path: &Path| -> Option<Achado> {
        let rest = path.strip_prefix(root).ok()?;
        if (!busca.ocultos && hidden(rest)) || exclude.iter().any(|e| path.starts_with(e)) || VIRTUAIS.iter().any(|v| path.starts_with(v)) { return None; }
        if rest.components().any(|c| SKIP.iter().any(|s| c.as_os_str() == *s)) { return None; }
        let name = path.file_name()?.to_string_lossy().into_owned();
        let folded = fold(&name);
        if !words.iter().all(|w| folded.contains(w.as_str())) { return None; }
        let meta = fs::metadata(path).ok()?;
        let folder = meta.is_dir();
        if !busca.tipos.is_empty() {
            let tipo = if folder { Some(Tipo::Pasta) } else { tipo_de(path) };
            if !tipo.is_some_and(|t| busca.tipos.contains(&t)) { return None; }
        }
        let changed = modified(&meta);
        if since.is_some_and(|s| changed < s) { return None; }
        Some(Achado { path: path.to_string_lossy().into_owned(), name, folder, bytes: if folder { 0 } else { meta.len() }, modified: changed })
    };
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    let mut push = |path: PathBuf, found: &mut Vec<Achado>| {
        if found.len() >= busca.limit * 4 || !seen.insert(path.clone()) { return; }
        if let Some(item) = accept(&path) { found.push(item); }
    };
    // Índice do sistema: rápido e cobre a pasta pessoal inteira (atualizado uma vez por dia).
    if let Some(longest) = words.iter().max_by_key(|w| w.len()) {
        if let Ok(out) = Command::new("plocate").args(["-i", "-l", "20000", "--"]).arg(longest).output() {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                let path = PathBuf::from(line);
                if path.starts_with(root) { push(path, &mut found); }
            }
        }
    }
    // O que é mais novo que o índice: varredura das pastas do dia a dia, com limite de tempo.
    let started = Instant::now();
    let mut pending: Vec<PathBuf> = if words.is_empty() || found.is_empty() { fresh.iter().filter(|f| f.starts_with(root)).cloned().collect() } else { fresh.to_vec() };
    let mut visited = 0usize;
    while let Some(dir) = pending.pop() {
        if started.elapsed() > Duration::from_secs(3) || visited > 200_000 { break; }
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            visited += 1;
            let name = entry.file_name();
            let text = name.to_string_lossy();
            if (!busca.ocultos && text.starts_with('.')) || SKIP.contains(&text.as_ref()) { continue; }
            let path = entry.path();
            if entry.file_type().is_ok_and(|t| t.is_dir()) { pending.push(path.clone()); }
            push(path, &mut found);
        }
    }
    let exact: Vec<String> = words.clone();
    found.sort_by(|a, b| {
        let score = |item: &Achado| { let stem = fold(item.name.rsplit_once('.').map_or(&item.name, |(s, _)| s)); i32::from(stem == exact.join(" ")) };
        score(b).cmp(&score(a)).then(b.modified.cmp(&a.modified))
    });
    found.truncate(busca.limit);
    found
}

// ---------------------------------------------------------------------------------------------
// Organizar: planos com prévia, Aplicar e desfazer

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "acao", rename_all = "snake_case")]
pub enum Passo {
    /// Criar uma pasta.
    Pasta { caminho: String },
    /// Mover ou renomear (o destino é o caminho completo novo).
    Mover { de: String, para: String },
    /// Mandar para a lixeira do sistema.
    Lixeira { caminho: String },
    /// Tirar um arquivo guardado na Iris (dados da Ayla) e pôr numa pasta liberada. A origem vem
    /// sempre do registro da Ayla pelo `arquivo` (id), nunca de um caminho recebido.
    Iris {
        arquivo: String,
        nome: String,
        para: String,
        /// Preenchido ao aplicar, para desfazer: o registro como estava na Iris.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        registro: Option<IrisRegistro>,
    },
}

/// Registro de um arquivo da Iris, guardado no histórico para voltar ao lugar ao desfazer.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct IrisRegistro {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub bytes: i64,
    pub created_at: i64,
    pub conversation_id: Option<String>,
    pub project_id: Option<String>,
    pub message_id: Option<i64>,
    /// Créditos da imagem baixada (fonte, autor, licença), que ficavam ao lado do arquivo.
    #[serde(default)]
    pub creditos: Option<String>,
}

/// O lado da Ayla nos passos da Iris (registro e pasta de dados), fora deste módulo.
pub trait IrisOrigem {
    /// Caminho do arquivo guardado e o registro dele.
    fn tirar(&self, id: &str) -> Result<(PathBuf, IrisRegistro), String>;
    /// Depois de mover: o arquivo sai da Iris (registro e pasta dele nos dados da Ayla).
    fn esquecer(&self, registro: &IrisRegistro) -> Result<(), String>;
    /// Desfazer: o arquivo em `de` volta para a Iris com o mesmo registro.
    fn devolver(&self, de: &Path, registro: &IrisRegistro) -> Result<(), String>;
}

/// Destino livre ao lado, sem repetir os já escolhidos neste plano.
fn unique_target(to: &Path, taken: &mut HashSet<PathBuf>) -> PathBuf {
    let mut target = free_name(to);
    if taken.contains(&target) {
        let stem = to.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = to.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        let dir = to.parent().unwrap_or(Path::new("/"));
        target = (2..).map(|n| dir.join(format!("{stem} {n}{ext}"))).find(|p| !p.exists() && !taken.contains(p)).unwrap_or(target);
    }
    taken.insert(target.clone());
    target
}

/// Plano para levar arquivos da Iris (id, nome) para `pasta`, sem sobrescrever nada.
pub fn plan_from_iris(arquivos: &[(String, String)], pasta: &Path) -> Vec<Passo> {
    let mut taken = HashSet::new();
    let mut steps: Vec<Passo> = arquivos.iter().map(|(id, nome)| {
        let para = unique_target(&pasta.join(nome), &mut taken);
        Passo::Iris { arquivo: id.clone(), nome: nome.clone(), para: para.to_string_lossy().into_owned(), registro: None }
    }).collect();
    if !pasta.exists() { steps.insert(0, Passo::Pasta { caminho: pasta.to_string_lossy().into_owned() }); }
    steps
}

/// Os últimos guardados na Iris, do mais novo para o mais antigo: os `quantos` pedidos ou, sem
/// número, a leva mais recente (salvos com até 2 h entre um e outro), no máximo 60.
pub fn recentes(arquivos: &[(String, i64)], quantos: Option<usize>) -> Vec<String> {
    let mut sorted: Vec<&(String, i64)> = arquivos.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    if let Some(n) = quantos { return sorted.into_iter().take(n).map(|(id, _)| id.clone()).collect(); }
    let mut out = Vec::new();
    let mut last: Option<i64> = None;
    for (id, when) in sorted {
        if last.is_some_and(|l| l - when > 2 * 3600) || out.len() >= 60 { break; }
        out.push(id.clone());
        last = Some(*when);
    }
    out
}

/// Pasta de destino dita na conversa ("wallpapers", "área de trabalho"): uma das liberadas ou uma
/// subpasta delas (até 3 níveis, a mais rasa primeiro), aceitando singular e plural.
pub fn pasta_destino(nome: &str, pastas: &[PathBuf]) -> Option<PathBuf> {
    if let Some(folder) = pasta_pelo_nome(nome, pastas) { return Some(folder); }
    let wanted = fold(nome.trim());
    if wanted.is_empty() { return None; }
    let mut names = vec![wanted.clone(), wanted.trim_end_matches('s').to_string(), format!("{}s", wanted.trim_end_matches('s'))];
    if ["wallpaper", "wallpapers", "papel de parede", "papeis de parede", "planos de fundo", "plano de fundo"].contains(&wanted.as_str()) {
        names.extend(["wallpaper", "wallpapers", "papel de parede", "papeis de parede", "planos de fundo", "plano de fundo", "backgrounds"].map(String::from));
    }
    let mut level: Vec<PathBuf> = pastas.to_vec();
    let mut visited = 0usize;
    for _ in 0..3 {
        let mut next = Vec::new();
        let mut found: Vec<PathBuf> = Vec::new();
        for dir in &level {
            let Ok(entries) = fs::read_dir(dir) else { continue };
            for entry in entries.flatten() {
                visited += 1;
                if visited > 20_000 { break; }
                if !entry.file_type().is_ok_and(|t| t.is_dir()) { continue; }
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || SKIP.contains(&name.as_str()) { continue; }
                if names.contains(&fold(&name)) { found.push(entry.path()); }
                next.push(entry.path());
            }
        }
        found.sort();
        if let Some(first) = found.into_iter().next() { return Some(first); }
        level = next;
    }
    None
}

/// Nome livre ao lado: "foto.png" → "foto 2.png" se já existir.
pub fn free_name(path: &Path) -> PathBuf {
    if !path.exists() { return path.to_path_buf(); }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let dir = path.parent().unwrap_or(Path::new("/"));
    (2..).map(|n| dir.join(format!("{stem} {n}{ext}"))).find(|p| !p.exists()).unwrap_or_else(|| path.to_path_buf())
}

fn categoria(tipo: Tipo) -> &'static str {
    match tipo {
        Tipo::Imagem => "Imagens", Tipo::Video => "Vídeos", Tipo::Audio => "Áudio", Tipo::Pdf => "PDFs",
        Tipo::Documento => "Documentos", Tipo::Compactado => "Compactados", Tipo::Instalador => "Instaladores", Tipo::Pasta => "Pastas",
    }
}

/// Arquivos soltos da pasta (sem entrar nas subpastas), fora de ocultos e downloads em andamento.
fn loose_files(folder: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(folder).into_iter().flatten().flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            !name.starts_with('.') && !name.ends_with(".part") && !name.ends_with(".crdownload") && !name.ends_with(".download")
        })
        .collect();
    files.sort();
    files
}

/// Destinos sem colisão entre si nem com o que já existe.
fn plan_moves(moves: Vec<(PathBuf, PathBuf)>) -> Vec<Passo> {
    let mut folders = Vec::new();
    let mut taken: HashSet<PathBuf> = HashSet::new();
    let mut steps = Vec::new();
    for (from, to) in moves {
        if let Some(dir) = to.parent() {
            if !dir.exists() && !folders.contains(&dir.to_path_buf()) { folders.push(dir.to_path_buf()); }
        }
        let target = unique_target(&to, &mut taken);
        steps.push(Passo::Mover { de: from.to_string_lossy().into_owned(), para: target.to_string_lossy().into_owned() });
    }
    folders.into_iter().map(|f| Passo::Pasta { caminho: f.to_string_lossy().into_owned() }).chain(steps).collect()
}

/// Separa os arquivos soltos em pastas por tipo (Imagens, PDFs, Documentos…). Os sem tipo ficam.
pub fn plan_by_type(folder: &Path) -> Vec<Passo> {
    plan_moves(loose_files(folder).into_iter()
        .filter_map(|file| { let tipo = tipo_de(&file)?; Some((file.clone(), folder.join(categoria(tipo)).join(file.file_name()?))) })
        .collect())
}

/// Separa os arquivos soltos em pastas por ano e mês da alteração ("2026/11").
pub fn plan_by_date(folder: &Path) -> Vec<Passo> {
    use chrono::{Datelike, TimeZone};
    plan_moves(loose_files(folder).into_iter()
        .filter_map(|file| {
            let when = chrono::Local.timestamp_opt(modified(&fs::metadata(&file).ok()?), 0).single()?;
            Some((file.clone(), folder.join(when.year().to_string()).join(format!("{:02}", when.month())).join(file.file_name()?)))
        })
        .collect())
}

/// Confere o plano antes de qualquer mudança: tudo dentro das pastas liberadas, origens existindo,
/// destinos livres e nada mexendo na própria pasta liberada.
pub fn check(plan: &[Passo], allowed: &[PathBuf]) -> Result<(), String> {
    if plan.is_empty() { return Err("Nada a fazer".into()); }
    if plan.len() > 5000 { return Err("Plano grande demais (até 5.000 passos)".into()); }
    let outside = |p: &Path| format!("{} está fora das pastas liberadas para o Hand", p.display());
    let mut created: HashSet<PathBuf> = HashSet::new();
    let mut targets: HashSet<PathBuf> = HashSet::new();
    for step in plan {
        match step {
            Passo::Pasta { caminho } => {
                let path = PathBuf::from(caminho);
                if !inside(&path, allowed) { return Err(outside(&path)); }
                created.insert(path);
            }
            Passo::Mover { de, para } => {
                let (from, to) = (PathBuf::from(de), PathBuf::from(para));
                if !inside(&from, allowed) { return Err(outside(&from)); }
                if !inside(&to, allowed) { return Err(outside(&to)); }
                if allowed.contains(&from) { return Err(format!("{} é uma pasta liberada e não pode ser movida", from.display())); }
                if fs::symlink_metadata(&from).is_err() { return Err(format!("{} não existe mais", from.display())); }
                if to.exists() || !targets.insert(to.clone()) { return Err(format!("{} já existe", to.display())); }
                if to.starts_with(&from) { return Err("Uma pasta não pode ir para dentro dela mesma".into()); }
                if let Some(dir) = to.parent() { if !dir.is_dir() && !created.contains(dir) { return Err(format!("A pasta {} não existe", dir.display())); } }
            }
            Passo::Lixeira { caminho } => {
                let path = PathBuf::from(caminho);
                if !inside(&path, allowed) { return Err(outside(&path)); }
                if allowed.contains(&path) { return Err(format!("{} é uma pasta liberada e não pode ir para a lixeira", path.display())); }
                if fs::symlink_metadata(&path).is_err() { return Err(format!("{} não existe mais", path.display())); }
            }
            Passo::Iris { arquivo, nome, para, .. } => {
                let to = PathBuf::from(para);
                if arquivo.is_empty() || nome.is_empty() { return Err("Arquivo da Iris inválido".into()); }
                if !inside(&to, allowed) { return Err(outside(&to)); }
                if to.exists() || !targets.insert(to.clone()) { return Err(format!("{} já existe", to.display())); }
                if let Some(dir) = to.parent() { if !dir.is_dir() && !created.contains(dir) { return Err(format!("A pasta {} não existe", dir.display())); } }
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Lote {
    pub id: String,
    pub quando: i64,
    pub titulo: String,
    /// Passos feitos, na ordem (só os que deram certo).
    pub feitos: Vec<Passo>,
    #[serde(default)]
    pub desfeito: bool,
    /// Cartão da conversa que pediu (para ele voltar como aplicado ao reabrir).
    #[serde(default)]
    pub origem: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Resultado {
    pub lote: Lote,
    /// Passos que não deram certo, com o motivo.
    pub falhas: Vec<(Passo, String)>,
}

pub fn move_path(from: &Path, to: &Path) -> std::io::Result<()> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        // Outro disco: copia e apaga (só arquivos; pastas entre discos ficam de fora).
        Err(_) if from.is_file() => { fs::copy(from, to)?; fs::remove_file(from) }
        Err(error) => Err(error),
    }
}

fn trash(path: &Path) -> Result<(), String> {
    let ok = Command::new("gio").arg("trash").arg(path).status().map_err(|_| "Para usar a lixeira, instale o gio (glib2)")?.success();
    if ok { Ok(()) } else { Err("A lixeira do sistema recusou".into()) }
}

/// Executa um plano já conferido. Para no meio só o passo que falhar; o resto segue.
pub fn apply(plan: &[Passo], allowed: &[PathBuf], titulo: &str, iris: Option<&dyn IrisOrigem>) -> Result<Resultado, String> {
    check(plan, allowed)?;
    if plan.iter().any(|p| matches!(p, Passo::Iris { .. })) && iris.is_none() { return Err("Arquivos da Iris só pela Ayla".into()); }
    let mut feitos = Vec::new();
    let mut falhas = Vec::new();
    for step in plan {
        if let (Passo::Iris { arquivo, nome, para, .. }, Some(iris)) = (step, iris) {
            let done = (|| {
                if Path::new(para).exists() { return Err("o destino já existe".to_string()); }
                let (from, registro) = iris.tirar(arquivo)?;
                move_path(&from, Path::new(para)).map_err(|e| e.to_string())?;
                if let Err(error) = iris.esquecer(&registro) {
                    // Sem atualizar o registro, o arquivo volta para não sumir da Iris.
                    let _ = move_path(Path::new(para), &from);
                    return Err(error);
                }
                Ok(registro)
            })();
            match done {
                Ok(registro) => feitos.push(Passo::Iris { arquivo: arquivo.clone(), nome: nome.clone(), para: para.clone(), registro: Some(registro) }),
                Err(error) => falhas.push((step.clone(), error)),
            }
            continue;
        }
        let done = match step {
            Passo::Iris { .. } => unreachable!("conferido acima"),
            Passo::Pasta { caminho } => fs::create_dir_all(caminho).map_err(|e| e.to_string()),
            Passo::Mover { de, para } => {
                if Path::new(para).exists() { Err("o destino já existe".to_string()) } else { move_path(Path::new(de), Path::new(para)).map_err(|e| e.to_string()) }
            }
            Passo::Lixeira { caminho } => trash(Path::new(caminho)),
        };
        match done { Ok(()) => feitos.push(step.clone()), Err(error) => falhas.push((step.clone(), error)) }
    }
    let id = format!("{}-{}", now_secs(), std::process::id() ^ (feitos.len() as u32));
    Ok(Resultado { lote: Lote { id, quando: now_secs(), titulo: titulo.to_string(), feitos, desfeito: false, origem: None }, falhas })
}

/// Desfaz um lote: devolve o que foi movido (do último para o primeiro) e apaga as pastas criadas
/// que ficaram vazias. O que foi para a lixeira volta pela lixeira do sistema.
pub fn undo(lote: &Lote, iris: Option<&dyn IrisOrigem>) -> (usize, Vec<String>) {
    let mut back = 0;
    let mut problems = Vec::new();
    for step in lote.feitos.iter().rev() {
        match step {
            Passo::Iris { para, registro, nome, .. } => {
                let to = Path::new(para);
                match (registro, iris) {
                    _ if !to.exists() => problems.push(format!("{} não está mais lá", to.display())),
                    (Some(registro), Some(iris)) => match iris.devolver(to, registro) { Ok(()) => back += 1, Err(e) => problems.push(format!("{nome}: {e}")) },
                    _ => problems.push(format!("{nome}: não dá para devolver à Iris")),
                }
            }
            Passo::Mover { de, para } => {
                let (from, to) = (Path::new(de), Path::new(para));
                if from.exists() { problems.push(format!("{} já existe de novo", from.display())); continue; }
                if !to.exists() { problems.push(format!("{} não está mais lá", to.display())); continue; }
                match move_path(to, from) { Ok(()) => back += 1, Err(e) => problems.push(format!("{}: {e}", to.display())) }
            }
            Passo::Pasta { caminho } => { let _ = fs::remove_dir(caminho); }
            Passo::Lixeira { caminho } => problems.push(format!("{} está na lixeira do sistema", caminho)),
        }
    }
    (back, problems)
}

/// Histórico dos lotes (os 50 mais recentes), num arquivo JSON nos dados da Ayla.
pub fn history_load(path: &Path) -> Vec<Lote> {
    fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn history_save(path: &Path, lotes: &[Lote]) -> Result<(), String> {
    if let Some(dir) = path.parent() { fs::create_dir_all(dir).map_err(|_| "Não foi possível guardar o histórico do Hand")?; }
    let start = lotes.len().saturating_sub(50);
    let text = serde_json::to_string_pretty(&lotes[start..]).map_err(|_| "Histórico inválido")?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text).and_then(|_| fs::rename(&tmp, path)).map_err(|_| "Não foi possível guardar o histórico do Hand".into())
}

// ---------------------------------------------------------------------------------------------
// Programas e comandos

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Programa {
    pub id: String,
    pub name: String,
    pub comment: String,
    pub icon: String,
}

pub fn application_dirs() -> Vec<PathBuf> {
    let home = home();
    let data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
    let mut dirs = vec![data_home.join("applications"), data_home.join("flatpak/exports/share/applications")];
    let system = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    dirs.extend(system.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join("applications")));
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
    dirs
}

/// Lê um `.desktop` (só a seção principal, nome em português quando houver).
pub fn parse_desktop(id: &str, text: &str) -> Option<Programa> {
    let mut main = false;
    // Português do Brasil antes do de Portugal ("Arquivos", não "Ficheiros").
    let (mut name, mut name_br, mut name_pt, mut comment, mut comment_br, mut comment_pt, mut icon) = (None, None, None, None, None, None, String::new());
    let (mut app, mut shown) = (false, true);
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') { main = line == "[Desktop Entry]"; continue; }
        if !main { continue; }
        let Some((key, value)) = line.split_once('=') else { continue };
        match key.trim() {
            "Type" => app = value.trim() == "Application",
            "NoDisplay" | "Hidden" if value.trim() == "true" => shown = false,
            "Name" => name = Some(value.trim().to_string()),
            "Name[pt_BR]" => name_br = Some(value.trim().to_string()),
            "Name[pt]" => name_pt = Some(value.trim().to_string()),
            "Comment" => comment = Some(value.trim().to_string()),
            "Comment[pt_BR]" => comment_br = Some(value.trim().to_string()),
            "Comment[pt]" => comment_pt = Some(value.trim().to_string()),
            "Icon" => icon = value.trim().to_string(),
            _ => {}
        }
    }
    (app && shown).then_some(())?;
    Some(Programa { id: id.to_string(), name: name_br.or(name_pt).or(name)?, comment: comment_br.or(comment_pt).or(comment).unwrap_or_default(), icon })
}

/// Programas instalados (os que aparecem no menu), sem repetir o mesmo id.
pub fn programs() -> Vec<Programa> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for dir in application_dirs() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "desktop") { continue; }
            let id = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            if !seen.insert(id.clone()) { continue; }
            if let Some(program) = fs::read_to_string(&path).ok().and_then(|t| parse_desktop(&id, &t)) { out.push(program); }
        }
    }
    out.sort_by_key(|a| fold(&a.name));
    out
}

/// Abre um programa pelo id do `.desktop` (como pelo menu), sem esperar ele fechar.
pub fn launch(id: &str) -> Result<(), String> {
    if id.is_empty() || id.contains('/') || !programs().iter().any(|p| p.id == id) { return Err("Programa não encontrado".into()); }
    Command::new("gtk-launch").arg(id).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn()
        .map(|_| ()).map_err(|_| "Não foi possível abrir o programa (falta o gtk-launch)".into())
}

/// Abre um arquivo ou pasta no programa padrão, ou mostra o arquivo na pasta dele.
pub fn open_path(path: &Path, reveal: bool) -> Result<(), String> {
    if !path.exists() { return Err("Arquivo não encontrado".into()); }
    let target = if reveal && !path.is_dir() { path.parent().unwrap_or(path) } else { path };
    Command::new("xdg-open").arg(target).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn()
        .map(|_| ()).map_err(|_| "Não foi possível abrir (falta o xdg-open)".into())
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Saida {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub cortado: bool,
    pub tempo_esgotado: bool,
}

const MAX_OUTPUT: usize = 64 * 1024;

/// Roda um comando no shell (na pasta pessoal), com limite de tempo e de saída. Só depois da
/// confirmação da pessoa: a página e os cartões da conversa mostram o comando antes.
pub async fn run(command: &str, cwd: &Path, timeout: Duration) -> Result<Saida, String> {
    if command.trim().is_empty() { return Err("Comando vazio".into()); }
    let child = tokio::process::Command::new("bash").arg("-lc").arg(command).current_dir(cwd)
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped())
        .kill_on_drop(true).spawn().map_err(|_| "Não foi possível rodar o comando")?;
    let cut = |bytes: &[u8]| { let text = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_OUTPUT)]).into_owned(); (text, bytes.len() > MAX_OUTPUT) };
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => {
            let ((stdout, a), (stderr, b)) = (cut(&out.stdout), cut(&out.stderr));
            Ok(Saida { code: out.status.code(), stdout, stderr, cortado: a || b, tempo_esgotado: false })
        }
        Ok(Err(_)) => Err("O comando parou".into()),
        Err(_) => Ok(Saida { code: None, stdout: String::new(), stderr: String::new(), cortado: false, tempo_esgotado: true }),
    }
}

// ---------------------------------------------------------------------------------------------
// Pedidos na conversa (sem modelo): "tira um print", "acha o PDF do contrato", "organiza meus
// Downloads", "abre o Firefox", "roda o comando `df -h`".

#[derive(Clone, Debug, PartialEq)]
pub enum Pedido {
    Captura(Captura),
    Busca { consulta: String, tipo: Option<Tipo>, dias: Option<u32> },
    Organizar { pasta: String, por_data: bool },
    Programa(String),
    Comando(String),
    /// Tarefa de vários passos na tela ("abre o Firefox e entra no meu e-mail"): plano para aprovar.
    Tarefa(String),
    /// Levar o que foi guardado na Iris para uma pasta ("põe os últimos wallpapers que salvei na
    /// Iris na minha pasta de wallpapers"): plano com prévia, sem usar a tela.
    DaIris { pasta: String, quantos: Option<usize>, tipo: Tipo },
    /// Trocar o papel de parede do sistema ("define um deles como wallpaper"): a imagem sai do
    /// texto, de uma pasta citada ou do que a conversa acabou de mover ou achar.
    PapelDeParede(String),
}

/// Pedido para tirar arquivos da Iris e pôr numa pasta. A pasta vem depois de "pasta" ou da última
/// preposição depois de "Iris" ("… da Iris em Wallpapers").
fn pedido_iris(text: &str, w: &[String]) -> Option<Pedido> {
    // "na Iris", ou o que a pessoa salvou/baixou "na Ayla" (fica na Iris); "Ayla, …" é só chamar.
    let at = w.iter().position(|x| x == "iris").or_else(|| w.iter().enumerate().skip(1)
        .find(|(i, x)| *x == "ayla" && ["na", "da", "pela", "pelo", "com", "no"].contains(&w[i - 1].as_str()))
        .filter(|_| w.iter().any(|x| ["baixei", "salvei", "guardei", "baixou", "salvou", "guardou", "baixados", "salvos", "guardados", "baixadas", "salvas", "guardadas"].contains(&x.as_str())))
        .map(|(i, _)| i))?;
    let verbs = ["organiza", "organize", "organizar", "move", "mova", "mover", "manda", "mande", "mandar", "coloca", "coloque", "colocar", "poe", "ponha",
        "passa", "passe", "passar", "leva", "leve", "levar", "joga", "jogue", "transfere", "transfira", "guarda", "guarde", "tira", "tire", "salva", "salve", "separa", "separe"];
    if !w.iter().any(|x| verbs.contains(&x.as_str())) { return None; }
    let stop = ["de", "da", "do", "dos", "das", "minha", "meu", "minhas", "meus", "a", "o", "as", "os", "nova", "novo"];
    let end = ["do", "no", "computador", "pc", "por", "favor", "pra", "para", "mim", "e", "que", "agora", "ai", "la", "aqui"];
    let start = match w.iter().rposition(|x| x == "pasta" || x == "pastas") {
        Some(p) => p + 1,
        None => w.iter().enumerate().skip(at + 1).filter(|(_, x)| ["para", "pra", "pro", "em", "na", "no", "nas", "nos"].contains(&x.as_str())).map(|(i, _)| i + 1).last()?,
    };
    let name: Vec<&str> = w[start.min(w.len())..].iter().map(String::as_str).skip_while(|x| stop.contains(x)).take_while(|x| !end.contains(x)).collect();
    if name.is_empty() || name.len() > 4 { return None; }
    let has = |list: &[&str]| w.iter().any(|x| list.contains(&x.as_str()));
    let tipo = if has(&["video", "videos"]) { Tipo::Video } else if has(&["pdf", "pdfs"]) { Tipo::Pdf } else { Tipo::Imagem };
    Some(Pedido::DaIris { pasta: name.join(" "), quantos: super::images::count(text), tipo })
}

/// "os wallpapers que eu baixei" sem dizer onde: pode ser pela Ayla (Iris) ou pelo navegador
/// (Downloads). Melhor perguntar que adivinhar e mover a pasta errada.
pub fn origem_ambigua(text: &str) -> bool {
    let w = words(text);
    let has = |list: &[&str]| w.iter().any(|x| list.contains(&x.as_str()));
    has(&["baixei", "salvei", "baixados", "baixadas", "baixou", "salvou"])
        && !has(&["downloads", "download", "navegador", "internet", "browser", "iris", "ayla", "site", "sites", "zen", "firefox", "chrome", "chromium", "celular", "torrent", "telegram", "whatsapp", "discord", "email", "e-mail"])
}

/// "define um deles como wallpaper", "troca o papel de parede", "põe essa de plano de fundo".
/// No singular: "move os wallpapers para a pasta" é mover arquivos; "cria um wallpaper", design.
fn pede_papel_de_parede(w: &[String]) -> bool {
    let at = |i: usize| w.get(i).map(String::as_str).unwrap_or("");
    let termo = (0..w.len()).any(|i| {
        let antes_pasta = i >= 1 && (at(i - 1) == "pasta" || (at(i - 1) == "de" && i >= 2 && at(i - 2) == "pasta"));
        let fala = at(i) == "wallpaper" || (at(i) == "papel" && at(i + 1) == "de" && at(i + 2) == "parede")
            || (at(i) == "plano" && at(i + 1) == "de" && at(i + 2) == "fundo") || (at(i) == "fundo" && ["de", "da", "do"].contains(&at(i + 1)) && ["tela", "desktop"].contains(&at(i + 2)));
        fala && !antes_pasta
    });
    let verbos = ["define", "defina", "definir", "coloca", "coloque", "colocar", "poe", "ponha", "por", "usa", "use", "usar", "troca", "troque", "trocar", "muda", "mude", "mudar",
        "deixa", "deixe", "deixar", "bota", "bote", "botar", "seta", "sete", "setar", "aplica", "aplique", "aplicar", "escolhe", "escolha", "escolher", "altera", "altere", "alterar", "coloquei", "quero"];
    let cria = ["cria", "crie", "criar", "faz", "faca", "fazer", "gera", "gere", "gerar", "desenha", "desenhe", "monta", "monte", "montar"];
    // Uma cor ("para azul") não é imagem: vira tarefa, com plano.
    let cores = ["cor", "azul", "preto", "preta", "branco", "branca", "vermelho", "vermelha", "verde", "amarelo", "amarela", "roxo", "roxa", "rosa", "cinza", "laranja", "marrom", "escuro", "escura", "claro", "clara"];
    termo && w.iter().any(|x| verbos.contains(&x.as_str())) && !w.iter().any(|x| cria.contains(&x.as_str()) || cores.contains(&x.as_str()))
}

/// Qual imagem já vem marcada entre as opções: pelo nome, "a primeira", "a última/mais recente"
/// (pela data) ou "qualquer uma" (sorteio); sem pista, a primeira.
pub fn escolher_imagem(text: &str, opcoes: &[PathBuf]) -> usize {
    pista_de_imagem(text, opcoes).unwrap_or(0)
}

/// A mensagem aponta uma das opções (pelo nome, posição, data ou sorteio)? Para "usa a noite"
/// logo depois do cartão do papel de parede.
pub fn pista_de_imagem(text: &str, opcoes: &[PathBuf]) -> Option<usize> {
    if opcoes.is_empty() { return None; }
    let w = words(text);
    let has = |list: &[&str]| w.iter().any(|x| list.contains(&x.as_str()));
    let comuns = ["wallpaper", "wallpapers", "papel", "parede", "plano", "fundo", "tela", "imagem", "foto", "png", "jpg", "jpeg", "webp", "pasta", "como", "esse", "essa", "deles", "delas", "uma", "para", "pra", "meu", "minha", "sistema", "computador"];
    let nomes: Vec<String> = opcoes.iter().map(|p| fold(&p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())).collect();
    let pedidas = ["usa", "use", "usar", "define", "defina", "coloca", "coloque", "poe", "ponha", "bota", "quero", "prefiro", "escolhe", "escolha", "troca", "troque", "muda", "mude", "agora", "entao", "melhor", "pode", "ser"];
    if let Some(i) = w.iter().filter(|x| x.chars().count() >= 3 && !comuns.contains(&x.as_str()) && !pedidas.contains(&x.as_str())).find_map(|x| nomes.iter().position(|n| n.contains(x.as_str()))) { return Some(i); }
    let data = |p: &PathBuf| fs::metadata(p).and_then(|m| m.modified()).ok();
    if has(&["ultima", "ultimo", "recente", "nova", "novo"]) {
        return (0..opcoes.len()).max_by_key(|i| data(&opcoes[*i]));
    }
    if has(&["aleatoria", "aleatorio", "qualquer", "sorteia", "sorteie", "surpreenda", "surpreende"]) {
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos() as usize).unwrap_or(0);
        return Some(n % opcoes.len());
    }
    if has(&["primeira", "primeiro"]) { return Some(0); }
    if has(&["segunda", "segundo"]) && opcoes.len() > 1 { return Some(1); }
    if has(&["terceira", "terceiro"]) && opcoes.len() > 2 { return Some(2); }
    None
}

/// A pasta citada no pedido ("… da pasta Wallpapers como …"): o nome depois de "pasta".
pub fn pasta_citada(text: &str) -> Option<String> {
    let w = words(text);
    let at = w.iter().rposition(|x| x == "pasta")?;
    let fim = ["como", "de", "do", "no", "na", "para", "pra", "e", "que", "o", "a", "meu", "minha", "sistema", "wallpaper", "papel", "plano", "fundo"];
    let nome: Vec<&str> = w[at + 1..].iter().map(String::as_str).skip_while(|x| ["de", "da", "do", "dos", "das", "minha", "meu"].contains(x)).take_while(|x| !fim.contains(x)).collect();
    (!nome.is_empty() && nome.len() <= 3).then(|| nome.join(" "))
}

/// Imagens soltas de uma pasta, das mais novas para as mais antigas (até `limite`).
pub fn imagens_da_pasta(pasta: &Path, limite: usize) -> Vec<PathBuf> {
    let mut list: Vec<(std::time::SystemTime, PathBuf)> = fs::read_dir(pasta).into_iter().flatten().flatten()
        .map(|e| e.path()).filter(|p| p.is_file() && tipo_de(p) == Some(Tipo::Imagem) && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
        .map(|p| (fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), p)).collect();
    list.sort_by(|a, b| b.0.cmp(&a.0));
    list.into_iter().take(limite).map(|(_, p)| p).collect()
}

// ---------------------------------------------------------------------------------------------
// Papel de parede: pelo programa que o sistema já usa (nada novo instalado).

#[derive(Clone, Debug, PartialEq)]
pub enum Fundo {
    /// illogical-impulse (Quickshell): troca a imagem e gera as cores do tema.
    Quickshell(PathBuf),
    Swww,
    Hyprpaper,
    Plasma,
    Gnome,
}

impl Fundo {
    pub fn nome(&self) -> &'static str {
        match self { Fundo::Quickshell(_) => "Quickshell (illogical-impulse)", Fundo::Swww => "swww", Fundo::Hyprpaper => "hyprpaper", Fundo::Plasma => "Plasma", Fundo::Gnome => "GNOME" }
    }
    /// Programa e argumentos para pôr `imagem` como papel de parede.
    pub fn comando(&self, imagem: &Path) -> (String, Vec<String>) {
        let img = imagem.to_string_lossy().into_owned();
        match self {
            Fundo::Quickshell(script) => ("bash".into(), vec![script.to_string_lossy().into_owned(), "--image".into(), img]),
            Fundo::Swww => ("swww".into(), vec!["img".into(), img]),
            Fundo::Hyprpaper => ("hyprctl".into(), vec!["hyprpaper".into(), "reload".into(), format!(",{img}")]),
            Fundo::Plasma => ("plasma-apply-wallpaperimage".into(), vec![img]),
            Fundo::Gnome => ("sh".into(), vec!["-c".into(), "gsettings set org.gnome.desktop.background picture-uri \"file://$1\" && gsettings set org.gnome.desktop.background picture-uri-dark \"file://$1\"".into(), "sh".into(), img]),
        }
    }
}

fn no_path(programa: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(programa).is_file()))
}

fn rodando(nome: &str) -> bool {
    std::process::Command::new("pgrep").args(["-x", nome]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

/// O jeito de trocar o papel de parede neste computador, na ordem do mais específico.
pub fn fundo_do_sistema() -> Option<Fundo> {
    let script = home().join(".config/quickshell/ii/scripts/colors/switchwall.sh");
    if script.is_file() { return Some(Fundo::Quickshell(script)); }
    if no_path("swww") && (rodando("swww-daemon") || rodando("awww-daemon")) { return Some(Fundo::Swww); }
    if rodando("hyprpaper") { return Some(Fundo::Hyprpaper); }
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    if desktop.contains("kde") && no_path("plasma-apply-wallpaperimage") { return Some(Fundo::Plasma); }
    if desktop.contains("gnome") && no_path("gsettings") { return Some(Fundo::Gnome); }
    if no_path("swww") { return Some(Fundo::Swww); }
    None
}

/// O papel de parede de agora, quando o sistema conta (para "Voltar ao anterior").
pub fn fundo_atual(fundo: &Fundo) -> Option<PathBuf> {
    let path = match fundo {
        Fundo::Quickshell(_) => {
            let config: serde_json::Value = serde_json::from_slice(&fs::read(home().join(".config/illogical-impulse/config.json")).ok()?).ok()?;
            PathBuf::from(config["background"]["wallpaperPath"].as_str()?)
        }
        Fundo::Swww => {
            let out = std::process::Command::new("swww").arg("query").output().ok()?;
            let text = String::from_utf8_lossy(&out.stdout).into_owned();
            PathBuf::from(text.lines().next()?.split("image: ").nth(1)?.trim())
        }
        _ => return None,
    };
    path.is_file().then_some(path)
}

/// Põe `imagem` como papel de parede. Roda solto (o script do Quickshell pode ficar esperando
/// uma notificação de "Upscale?"), sem prender a Ayla.
pub fn definir_fundo(fundo: &Fundo, imagem: &Path) -> Result<(), String> {
    if !imagem.is_file() || tipo_de(imagem) != Some(Tipo::Imagem) { return Err("Essa imagem não existe mais".into()); }
    use std::os::unix::process::CommandExt;
    let (programa, args) = fundo.comando(imagem);
    std::process::Command::new(programa).args(args).current_dir(home())
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .process_group(0).spawn().map(|_| ()).map_err(|_| format!("Não consegui trocar o papel de parede pelo {}", fundo.nome()))
}

fn words(text: &str) -> Vec<String> {
    fold(text).split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != '.').filter(|w| !w.is_empty()).map(str::to_string).collect()
}

/// Lê o pedido. Só reconhece frases claras; o resto segue para a conversa normal.
pub fn pedido(message: &str) -> Option<Pedido> {
    let text = message.trim();
    if text.is_empty() || text.len() > 300 { return None; }
    let plain = fold(text);
    let w = words(text);
    let has = |list: &[&str]| w.iter().any(|x| list.contains(&x.as_str()));
    let first = |list: &[&str]| w.first().is_some_and(|x| list.contains(&x.as_str()))
        || (w.first().is_some_and(|x| ["ayla", "por", "pode", "voce", "me", "favor"].contains(&x.as_str())) && w.iter().take(4).any(|x| list.contains(&x.as_str())));
    // Arquivos da Iris para uma pasta: direto, sem a tela.
    if let Some(found) = pedido_iris(text, &w) { return Some(found); }
    if pede_papel_de_parede(&w) { return Some(Pedido::PapelDeParede(text.to_string())); }
    // Tarefa dita para o computador: "no computador, …", "Hand, …", "usa o computador para …".
    for prefix in ["no computador", "no pc", "hand", "usa o computador para", "use o computador para", "usando o computador"] {
        if plain.starts_with(prefix) && plain[prefix.len()..].starts_with([',', ':', ' ']) {
            // O resto, no texto original (com acentos): `fold` troca letra por letra.
            let rest: String = text.chars().skip(prefix.chars().count()).collect();
            let rest = rest.trim_start_matches([',', ':', ' ']).trim();
            // O que dá para fazer sem a tela (organizar, buscar, abrir um programa) vai direto.
            if let Some(direct) = pedido(rest).filter(|p| !matches!(p, Pedido::Tarefa(_))) { return Some(direct); }
            if rest.split_whitespace().count() >= 2 { return Some(Pedido::Tarefa(rest.to_string())); }
        }
    }
    // Comando entre crases: "roda o comando `df -h`".
    if first(&["roda", "rode", "rodar", "executa", "execute", "executar"]) {
        if let Some(start) = text.find('`') {
            let rest = &text[start + 1..];
            if let Some(end) = rest.find('`') { let command = rest[..end].trim(); if !command.is_empty() { return Some(Pedido::Comando(command.to_string())); } }
        }
        if let Some((_, command)) = text.split_once("comando:") { let command = command.trim(); if !command.is_empty() { return Some(Pedido::Comando(command.to_string())); } }
    }
    // Captura de tela.
    let shot = has(&["print", "printscreen", "screenshot", "screenshots", "prints"]) || plain.contains("captura de tela") || plain.contains("captura da tela");
    if shot && (first(&["tira", "tire", "tirar", "faz", "faca", "fazer", "bate", "bata", "captura", "capture", "salva", "salve", "da", "de"]) || w.len() <= 4) {
        let kind = if has(&["janela"]) { Captura::Janela } else if has(&["regiao", "parte", "pedaco", "area", "trecho", "recorte"]) { Captura::Regiao } else { Captura::Tela };
        return Some(Pedido::Captura(kind));
    }
    // Organizar uma pasta.
    if first(&["organiza", "organize", "organizar", "arruma", "arrume", "arrumar", "separa", "separe"]) {
        let por_data = plain.contains("por data") || plain.contains("por mes") || plain.contains("por ano");
        let stop = ["organiza", "organize", "organizar", "arruma", "arrume", "arrumar", "separa", "separe", "a", "o", "as", "os", "minha", "meu", "minhas", "meus", "pasta", "de", "da", "do", "por", "tipo", "data", "mes", "ano", "ayla", "pode", "voce", "me", "favor", "arquivos", "em", "pastas", "na", "no"];
        let name: Vec<&str> = w.iter().map(String::as_str).filter(|x| !stop.contains(x)).collect();
        if !name.is_empty() && name.len() <= 4 { return Some(Pedido::Organizar { pasta: name.join(" "), por_data }); }
        return None;
    }
    // Abrir um programa: o nome é conferido depois com os programas instalados.
    if first(&["abre", "abra", "abrir", "inicia", "inicie", "iniciar", "liga", "ligue"]) {
        let stop = ["abre", "abra", "abrir", "inicia", "inicie", "iniciar", "liga", "ligue", "o", "a", "os", "as", "programa", "aplicativo", "app", "pra", "para", "mim", "ayla", "pode", "voce", "me", "favor", "por"];
        if has(&["nota", "notas", "ficha", "conversa", "arquivo", "pasta", "pdf", "site", "link"]) { return None; }
        // "abre o Firefox e entra no meu e-mail": mais de uma coisa a fazer vira tarefa.
        if let Some(pos) = w.iter().position(|x| x == "e").filter(|p| *p >= 2 && w.len() > p + 2) {
            let name: Vec<&str> = w[..pos].iter().map(String::as_str).filter(|x| !stop.contains(x)).collect();
            if !name.is_empty() && name.len() <= 3 { return Some(Pedido::Tarefa(text.to_string())); }
        }
        let name: Vec<&str> = w.iter().map(String::as_str).filter(|x| !stop.contains(x)).collect();
        if !name.is_empty() && name.len() <= 3 { return Some(Pedido::Programa(name.join(" "))); }
        return None;
    }
    // Buscar arquivos: precisa falar de arquivo (para não confundir com pesquisa na internet).
    if first(&["procura", "procure", "procurar", "acha", "ache", "achar", "encontra", "encontre", "encontrar", "busca", "busque", "buscar", "localiza", "localize", "cade", "onde"]) {
        let tipo = if has(&["pasta", "pastas"]) { Some(Tipo::Pasta) } else if has(&["pdf", "pdfs"]) { Some(Tipo::Pdf) }
            else if has(&["imagem", "imagens", "foto", "fotos", "print", "prints"]) { Some(Tipo::Imagem) } else if has(&["video", "videos"]) { Some(Tipo::Video) }
            else if has(&["musica", "musicas", "audio", "audios"]) { Some(Tipo::Audio) } else if has(&["planilha", "planilhas", "documento", "documentos", "doc"]) { Some(Tipo::Documento) } else { None };
        let filey = tipo.is_some() || has(&["arquivo", "arquivos"]) || plain.contains("no meu computador") || plain.contains("no computador") || plain.contains("nos meus arquivos");
        if !filey || has(&["internet", "google", "site", "web", "youtube"]) { return None; }
        let dias = if has(&["hoje"]) { Some(1) } else if plain.contains("essa semana") || plain.contains("esta semana") || plain.contains("ultima semana") { Some(7) } else if plain.contains("esse mes") || plain.contains("este mes") || plain.contains("ultimo mes") { Some(30) } else { None };
        let stop = ["procura", "procure", "procurar", "acha", "ache", "achar", "encontra", "encontre", "encontrar", "busca", "busque", "buscar", "localiza", "localize", "cade", "onde", "esta", "fica", "ficou", "o", "a", "os", "as", "um", "uma", "de", "da", "do", "das", "dos", "meu", "minha", "meus", "minhas", "arquivo", "arquivos", "pasta", "pastas", "pdf", "pdfs", "imagem", "imagens", "foto", "fotos", "video", "videos", "musica", "musicas", "audio", "audios", "planilha", "planilhas", "documento", "documentos", "doc", "no", "na", "nos", "nas", "computador", "pc", "pra", "para", "mim", "ayla", "pode", "voce", "me", "favor", "por", "que", "chamado", "chamada", "com", "nome", "hoje", "essa", "este", "esse", "semana", "mes", "ultima", "ultimo", "salvei", "baixei", "e", "em"];
        let consulta: Vec<&str> = w.iter().map(String::as_str).filter(|x| !stop.contains(x)).collect();
        if consulta.is_empty() && dias.is_none() { return None; }
        return Some(Pedido::Busca { consulta: consulta.join(" "), tipo, dias });
    }
    None
}

/// Pasta liberada pelo nome dito na conversa ("downloads", "documentos", "área de trabalho").
pub fn pasta_pelo_nome(nome: &str, pastas: &[PathBuf]) -> Option<PathBuf> {
    let wanted = fold(nome);
    let aliases = |path: &Path| -> Vec<String> {
        let base = fold(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        let extra: &[&str] = match base.as_str() {
            "downloads" => &["download", "baixados"], "documents" => &["documentos", "documento"], "pictures" => &["imagens", "fotos"],
            "desktop" => &["area de trabalho", "area trabalho", "desktop"], "videos" => &["videos"], "music" => &["musicas", "musica"], _ => &[],
        };
        std::iter::once(base).chain(extra.iter().map(|s| s.to_string())).collect()
    };
    pastas.iter().find(|p| aliases(p).contains(&wanted)).cloned()
}

/// Programa pelo nome dito ("firefox", "arquivos", "terminal"): igual, ou palavra inteira no nome.
pub fn programa_pelo_nome<'a>(nome: &str, programas: &'a [Programa]) -> Option<&'a Programa> {
    let wanted = fold(nome);
    programas.iter().find(|p| fold(&p.name) == wanted || fold(&p.id) == wanted || fold(&p.id).rsplit('.').next() == Some(wanted.as_str()))
        .or_else(|| programas.iter().find(|p| words(&p.name).join(" ").split(' ').any(|w| w == wanted) && wanted.len() >= 3))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ayla-hand-{name}-{}-{}", std::process::id(), now_secs()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn organizar_por_tipo_mostra_o_plano_aplica_e_desfaz() {
        let root = temp("tipo");
        let downloads = root.join("Downloads");
        fs::create_dir_all(downloads.join("PDFs")).unwrap();
        for name in ["foto.png", "contrato.pdf", "notas.txt", "estranho.xyz", ".oculto.png", "filme.mp4.part"] { fs::write(downloads.join(name), name).unwrap(); }
        fs::write(downloads.join("PDFs/contrato.pdf"), "antigo").unwrap();
        let plan = plan_by_type(&downloads);
        let allowed = vec![downloads.clone()];
        assert!(plan.contains(&Passo::Pasta { caminho: downloads.join("Imagens").to_string_lossy().into() }));
        assert!(!plan.iter().any(|p| matches!(p, Passo::Pasta { caminho } if caminho.ends_with("PDFs"))), "pasta que já existe não é criada");
        assert!(plan.contains(&Passo::Mover { de: downloads.join("contrato.pdf").to_string_lossy().into(), para: downloads.join("PDFs/contrato 2.pdf").to_string_lossy().into() }), "nunca sobrescreve");
        assert!(!plan.iter().any(|p| matches!(p, Passo::Mover { de, .. } if de.contains("estranho") || de.contains("oculto") || de.ends_with(".part"))));
        let feito = apply(&plan, &allowed, "Downloads por tipo", None).unwrap();
        assert!(feito.falhas.is_empty());
        assert!(downloads.join("Imagens/foto.png").is_file() && downloads.join("Documentos/notas.txt").is_file());
        assert_eq!(fs::read_to_string(downloads.join("PDFs/contrato.pdf")).unwrap(), "antigo");
        let (back, problems) = undo(&feito.lote, None);
        assert_eq!((back, problems.len()), (3, 0));
        assert!(downloads.join("foto.png").is_file() && downloads.join("contrato.pdf").is_file());
        assert!(!downloads.join("Imagens").exists(), "pasta criada e vazia some ao desfazer");
        assert!(downloads.join("PDFs/contrato.pdf").is_file(), "o que já existia fica");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn plano_fora_das_pastas_liberadas_e_recusado() {
        let root = temp("fora");
        let (allowed_dir, other) = (root.join("Downloads"), root.join("Outra"));
        fs::create_dir_all(&allowed_dir).unwrap(); fs::create_dir_all(&other).unwrap();
        fs::write(other.join("a.txt"), "a").unwrap();
        fs::write(allowed_dir.join("b.txt"), "b").unwrap();
        let allowed = vec![allowed_dir.clone()];
        let s = |p: PathBuf| p.to_string_lossy().into_owned();
        assert!(check(&[Passo::Mover { de: s(other.join("a.txt")), para: s(allowed_dir.join("a.txt")) }], &allowed).is_err(), "origem fora");
        assert!(check(&[Passo::Mover { de: s(allowed_dir.join("b.txt")), para: s(other.join("b.txt")) }], &allowed).is_err(), "destino fora");
        assert!(check(&[Passo::Mover { de: s(allowed_dir.join("b.txt")), para: s(allowed_dir.join("../Outra/b.txt")) }], &allowed).is_err(), "sem ..");
        assert!(check(&[Passo::Mover { de: s(allowed_dir.join("b.txt")), para: s(allowed_dir.join(".escondido/b.txt")) }], &allowed).is_err(), "sem ocultas");
        assert!(check(&[Passo::Lixeira { caminho: s(allowed_dir.clone()) }], &allowed).is_err(), "a pasta liberada em si");
        fs::write(allowed_dir.join("c.txt"), "c").unwrap();
        assert!(check(&[Passo::Mover { de: s(allowed_dir.join("b.txt")), para: s(allowed_dir.join("c.txt")) }], &allowed).is_err(), "destino existente");
        assert!(check(&[Passo::Mover { de: s(allowed_dir.join("b.txt")), para: s(allowed_dir.join("Nova/b.txt")) }], &allowed).is_err(), "pasta de destino que não existe nem é criada");
        assert!(check(&[Passo::Pasta { caminho: s(allowed_dir.join("Nova")) }, Passo::Mover { de: s(allowed_dir.join("b.txt")), para: s(allowed_dir.join("Nova/b.txt")) }], &allowed).is_ok());
        assert_eq!(fs::read_to_string(other.join("a.txt")).unwrap(), "a", "nada mudou");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn busca_acha_pelo_nome_sem_acento_fora_de_ocultas_e_por_tipo() {
        let root = temp("busca");
        fs::create_dir_all(root.join("Docs/node_modules")).unwrap();
        fs::create_dir_all(root.join(".escondida")).unwrap();
        for p in ["Docs/Relatório Anual.pdf", "Docs/relatorio rascunho.txt", ".escondida/relatorio.pdf", "Docs/node_modules/relatorio.pdf", "Docs/outro.pdf"] { fs::write(root.join(p), "x").unwrap(); }
        let busca = |q: &str, tipo: Option<Tipo>| search(&Busca { query: q, tipos: tipo.as_slice(), ocultos: false, dias: None, limit: 20 }, &root, &[], &[root.clone()]).into_iter().map(|a| a.name).collect::<Vec<_>>();
        let mut nomes = busca("relatorio", None); nomes.sort();
        assert_eq!(nomes, vec!["Relatório Anual.pdf", "relatorio rascunho.txt"]);
        assert_eq!(busca("relatório", Some(Tipo::Pdf)), vec!["Relatório Anual.pdf"]);
        assert_eq!(busca("docs", Some(Tipo::Pasta)), vec!["Docs"]);
        assert!(busca("", None).is_empty(), "sem pedido, sem lista");
        let fora = search(&Busca { query: "relatorio", tipos: &[], ocultos: false, dias: None, limit: 20 }, &root, &[root.join("Docs")], &[root.clone()]);
        assert!(fora.is_empty(), "pasta excluída (dados da Ayla) não aparece");
        let opcoes = |tipos: &[Tipo], ocultos: bool| { let mut n: Vec<String> = search(&Busca { query: "relatorio", tipos, ocultos, dias: None, limit: 20 }, &root, &[], &[root.clone()]).into_iter().map(|a| a.name).collect(); n.sort(); n };
        assert_eq!(opcoes(&[Tipo::Pdf], true), vec!["Relatório Anual.pdf", "relatorio.pdf"], "com ocultos, acha o da pasta oculta");
        assert_eq!(opcoes(&[Tipo::Pdf, Tipo::Documento], false), vec!["Relatório Anual.pdf", "relatorio rascunho.txt"], "vários tipos de uma vez");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn programas_pelo_desktop_em_portugues_e_sem_ocultos() {
        let text = "[Desktop Entry]\nType=Application\nName=Files\nName[pt]=Ficheiros\nName[pt_BR]=Arquivos\nComment=Browse\nIcon=org.gnome.Nautilus\n[Desktop Action new]\nName=Nova janela\n";
        assert_eq!(parse_desktop("org.gnome.Nautilus", text), Some(Programa { id: "org.gnome.Nautilus".into(), name: "Arquivos".into(), comment: "Browse".into(), icon: "org.gnome.Nautilus".into() }));
        assert_eq!(parse_desktop("x", "[Desktop Entry]\nType=Application\nName=X\nNoDisplay=true\n"), None);
        assert_eq!(parse_desktop("y", "[Desktop Entry]\nType=Link\nName=Y\n"), None);
    }

    #[test]
    fn historico_guarda_os_50_mais_recentes() {
        let root = temp("historico");
        let path = root.join("hand/historico.json");
        let lotes: Vec<Lote> = (0..60).map(|i| Lote { id: i.to_string(), quando: i, titulo: String::new(), feitos: vec![], desfeito: false, origem: None }).collect();
        history_save(&path, &lotes).unwrap();
        let lidos = history_load(&path);
        assert_eq!((lidos.len(), lidos[0].id.as_str()), (50, "10"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn comando_com_saida_limite_de_tempo_e_codigo() {
        let saida = tauri::async_runtime::block_on(run("echo oi; echo erro >&2; exit 3", &std::env::temp_dir(), Duration::from_secs(5))).unwrap();
        assert_eq!((saida.code, saida.stdout.trim(), saida.stderr.trim()), (Some(3), "oi", "erro"));
        let lento = tauri::async_runtime::block_on(run("sleep 5", &std::env::temp_dir(), Duration::from_millis(200))).unwrap();
        assert!(lento.tempo_esgotado);
    }

    #[test]
    fn pedidos_da_conversa_so_frases_claras() {
        assert_eq!(pedido("tira um print da tela"), Some(Pedido::Captura(Captura::Tela)));
        assert_eq!(pedido("Ayla, tira um print dessa janela"), Some(Pedido::Captura(Captura::Janela)));
        assert_eq!(pedido("faz um screenshot de uma parte da tela"), Some(Pedido::Captura(Captura::Regiao)));
        assert_eq!(pedido("organiza meus Downloads"), Some(Pedido::Organizar { pasta: "downloads".into(), por_data: false }));
        assert_eq!(pedido("arruma a pasta Documentos por data"), Some(Pedido::Organizar { pasta: "documentos".into(), por_data: true }));
        assert_eq!(pedido("abre o Firefox"), Some(Pedido::Programa("firefox".into())));
        assert_eq!(pedido("abre a nota de ontem"), None, "notas são da Athena");
        assert_eq!(pedido("acha o PDF do contrato de aluguel"), Some(Pedido::Busca { consulta: "contrato aluguel".into(), tipo: Some(Tipo::Pdf), dias: None }));
        assert_eq!(pedido("onde está o arquivo relatório final?"), Some(Pedido::Busca { consulta: "relatorio final".into(), tipo: None, dias: None }));
        assert_eq!(pedido("procura as fotos que baixei hoje"), Some(Pedido::Busca { consulta: String::new(), tipo: Some(Tipo::Imagem), dias: Some(1) }));
        assert_eq!(pedido("procura na internet sobre baleias"), None, "pesquisa na internet não é busca de arquivo");
        assert_eq!(pedido("procura sobre a revolução francesa"), None);
        assert_eq!(pedido("roda o comando `df -h`"), Some(Pedido::Comando("df -h".into())));
        assert_eq!(pedido("como eu tiro um print no linux?"), None, "pergunta não é pedido");
        assert_eq!(pedido("qual a capital da França?"), None);
        assert_eq!(pedido("abre o Firefox e entra no meu e-mail"), Some(Pedido::Tarefa("abre o Firefox e entra no meu e-mail".into())));
        assert_eq!(pedido("No computador, muda o papel de parede para azul"), Some(Pedido::Tarefa("muda o papel de parede para azul".into())));
        assert_eq!(pedido("Hand: fecha todas as abas do navegador"), Some(Pedido::Tarefa("fecha todas as abas do navegador".into())));
        assert_eq!(pedido("No computador, põe o relatório na área de trabalho"), Some(Pedido::Tarefa("põe o relatório na área de trabalho".into())));
        assert_eq!(pedido("handebol é legal"), None, "\"hand\" só como prefixo separado");
        assert_eq!(pedido("abre o Firefox"), Some(Pedido::Programa("firefox".into())), "um programa só continua direto");
        assert_eq!(pedido("Hand, organiza meus Downloads"), Some(Pedido::Organizar { pasta: "downloads".into(), por_data: false }), "o que dá sem a tela vai direto");
    }

    #[test]
    fn pedidos_da_iris_para_uma_pasta() {
        let iris = |pasta: &str, quantos: Option<usize>| Some(Pedido::DaIris { pasta: pasta.into(), quantos, tipo: Tipo::Imagem });
        assert_eq!(pedido("organiza esses ultimos wallpapers que eu salvei no Iris na minha pasta de wallpapers do computador"), iris("wallpapers", None));
        assert_eq!(pedido("Hand, organiza esses últimos wallpapers que eu salvei no Iris na minha pasta de wallpapers"), iris("wallpapers", None));
        assert_eq!(pedido("move os últimos 5 wallpapers da Iris para Wallpapers"), iris("wallpapers", Some(5)));
        assert_eq!(pedido("coloca as fotos que salvei na Iris na área de trabalho"), iris("area de trabalho", None));
        assert_eq!(pedido("o que tem na iris?"), None, "pergunta sobre a Iris não move nada");
        // O que foi baixado ou salvo pela Ayla está na Iris.
        assert_eq!(pedido("move os wallpapers que eu baixei na ayla para a pasta wallpapers"), iris("wallpapers", None));
        assert_eq!(pedido("organiza esses wallpapers que salvei pela Ayla na minha pasta de wallpapers"), iris("wallpapers", None));
        assert_eq!(pedido("coloca as 3 imagens que baixei da ayla em Wallpapers"), iris("wallpapers", Some(3)));
        assert!(!matches!(pedido("Ayla, organiza meus Downloads"), Some(Pedido::DaIris { .. })), "chamar a Ayla não é falar da Iris");
        // Papel de parede: no singular e com um verbo de pôr; mover para a pasta e criar ficam fora.
        let fundo = |t: &str| matches!(pedido(t), Some(Pedido::PapelDeParede(_)));
        assert!(fundo("agora define um deles como wallpaper do sistema") && fundo("troca meu papel de parede pela última imagem da pasta Wallpapers"));
        assert!(fundo("coloca essa de plano de fundo") && fundo("Hand, muda o fundo de tela para a primeira"));
        assert!(!fundo("move os wallpapers para a pasta wallpaper") && !fundo("cria um wallpaper de outono") && !fundo("o que é um papel de parede?"));
        assert_eq!(pasta_citada("define a mais recente da pasta Wallpapers como papel de parede").as_deref(), Some("wallpapers"));
        let dir = std::env::temp_dir().join(format!("ayla-hand-fundo-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let (a, b, c) = (dir.join("Crimson Hair.png"), dir.join("lago.jpg"), dir.join("notas.txt"));
        for f in [&a, &b, &c] { fs::write(f, b"x").unwrap(); }
        let agora = SystemTime::now();
        fs::File::options().write(true).open(&a).unwrap().set_modified(agora - Duration::from_secs(60)).unwrap();
        fs::File::options().write(true).open(&b).unwrap().set_modified(agora).unwrap();
        let opcoes = imagens_da_pasta(&dir, 10);
        assert_eq!(opcoes, vec![b.clone(), a.clone()], "só imagens, das mais novas para as mais antigas");
        assert_eq!(escolher_imagem("põe a crimson de wallpaper", &opcoes), 1, "pelo nome");
        assert_eq!(escolher_imagem("define um deles como wallpaper", &opcoes), 0);
        assert_eq!(escolher_imagem("a mais recente como papel de parede", &[a.clone(), b.clone()]), 1, "pela data");
        assert_eq!(escolher_imagem("a segunda", &opcoes), 1);
        assert!(escolher_imagem("qualquer uma", &opcoes) < 2);
        assert_eq!(pista_de_imagem("usa a crimson", &opcoes), Some(1), "logo depois do cartão: pelo nome");
        assert_eq!(pista_de_imagem("obrigado", &opcoes), None);
        assert_eq!(pista_de_imagem("pode ser a primeira", &opcoes), Some(0));
        let _ = fs::remove_dir_all(&dir);
        let q = Fundo::Quickshell(PathBuf::from("/h/.config/quickshell/ii/scripts/colors/switchwall.sh"));
        assert_eq!(q.comando(Path::new("/h/Pictures/Wallpapers/a b.png")), ("bash".to_string(), vec!["/h/.config/quickshell/ii/scripts/colors/switchwall.sh".to_string(), "--image".into(), "/h/Pictures/Wallpapers/a b.png".into()]));
        assert_eq!(Fundo::Swww.comando(Path::new("/x.png")).1, vec!["img", "/x.png"]);
        assert!(definir_fundo(&Fundo::Swww, &dir.join("nao-existe.png")).is_err());
        assert!(origem_ambigua("move os wallpapers que eu baixei para a pasta wallpapers"));
        assert!(!origem_ambigua("move os wallpapers que baixei no navegador para a pasta wallpapers"));
        assert!(!origem_ambigua("move os wallpapers que baixei nos Downloads para Wallpapers") && !origem_ambigua("copia os pdfs da pasta Downloads para Documentos"));
    }

    #[test]
    fn iris_leva_a_leva_mais_recente_e_a_pasta_pelo_nome() {
        let arquivos: Vec<(String, i64)> = [("a", 10_000), ("b", 9_000), ("c", 1_000), ("d", 9_500)].iter().map(|(id, t)| (id.to_string(), *t)).collect();
        assert_eq!(recentes(&arquivos, None), vec!["a", "d", "b"], "para no intervalo de mais de 2 h");
        assert_eq!(recentes(&arquivos, Some(2)), vec!["a", "d"]);
        let root = temp("destino");
        let pictures = root.join("Pictures");
        fs::create_dir_all(pictures.join("Wallpapers")).unwrap();
        fs::create_dir_all(pictures.join(".oculta/wallpapers")).unwrap();
        fs::create_dir_all(root.join("Downloads/fundo/Wallpaper")).unwrap();
        let pastas = vec![root.join("Downloads"), pictures.clone()];
        assert_eq!(pasta_destino("wallpapers", &pastas), Some(pictures.join("Wallpapers")), "a mais rasa");
        assert_eq!(pasta_destino("papel de parede", &pastas), Some(pictures.join("Wallpapers")));
        assert_eq!(pasta_destino("imagens", &pastas), Some(pictures.clone()), "pasta liberada pelo apelido");
        assert_eq!(pasta_destino("musicas", &pastas), None);
        fs::remove_dir_all(&root).unwrap();
    }

    struct IrisFalsa { dados: PathBuf, registro: std::cell::RefCell<Vec<String>> }
    impl IrisOrigem for IrisFalsa {
        fn tirar(&self, id: &str) -> Result<(PathBuf, IrisRegistro), String> {
            if !self.registro.borrow().iter().any(|r| r == id) { return Err("não está na Iris".into()); }
            let name = format!("{id}.jpg");
            Ok((self.dados.join(id).join(&name), IrisRegistro { id: id.into(), name, kind: "image".into(), bytes: 1, created_at: 1, conversation_id: None, project_id: None, message_id: None, creditos: Some("{}".into()) }))
        }
        fn esquecer(&self, registro: &IrisRegistro) -> Result<(), String> {
            self.registro.borrow_mut().retain(|r| r != &registro.id);
            fs::remove_dir_all(self.dados.join(&registro.id)).map_err(|e| e.to_string())
        }
        fn devolver(&self, de: &Path, registro: &IrisRegistro) -> Result<(), String> {
            fs::create_dir_all(self.dados.join(&registro.id)).unwrap();
            fs::rename(de, self.dados.join(&registro.id).join(&registro.name)).map_err(|e| e.to_string())?;
            self.registro.borrow_mut().push(registro.id.clone());
            Ok(())
        }
    }

    #[test]
    fn mover_da_iris_aplica_e_desfaz() {
        let root = temp("iris");
        let (dados, destino) = (root.join("dados/files"), root.join("Pictures/Wallpapers"));
        for id in ["a", "b"] { fs::create_dir_all(dados.join(id)).unwrap(); fs::write(dados.join(id).join(format!("{id}.jpg")), id).unwrap(); }
        fs::create_dir_all(&destino).unwrap();
        fs::write(destino.join("a.jpg"), "antigo").unwrap();
        let iris = IrisFalsa { dados: dados.clone(), registro: std::cell::RefCell::new(vec!["a".into(), "b".into()]) };
        let plan = plan_from_iris(&[("a".into(), "a.jpg".into()), ("b".into(), "b.jpg".into())], &destino);
        assert!(matches!(&plan[0], Passo::Iris { para, .. } if para.ends_with("a 2.jpg")), "nunca sobrescreve");
        let allowed = vec![root.join("Pictures")];
        assert!(apply(&plan, &allowed, "Iris", None).is_err(), "sem a Ayla, nada da Iris");
        assert!(check(&plan, &[root.join("Downloads")]).is_err(), "destino fora das liberadas");
        let feito = apply(&plan, &allowed, "Iris → Wallpapers", Some(&iris)).unwrap();
        assert!(feito.falhas.is_empty());
        assert_eq!(fs::read_to_string(destino.join("a 2.jpg")).unwrap(), "a");
        assert_eq!(fs::read_to_string(destino.join("a.jpg")).unwrap(), "antigo");
        assert!(!dados.join("b").exists() && iris.registro.borrow().is_empty(), "saiu da Iris");
        assert!(feito.lote.feitos.iter().all(|p| matches!(p, Passo::Iris { registro: Some(_), .. })));
        let (back, problems) = undo(&feito.lote, Some(&iris));
        assert_eq!((back, problems.len()), (2, 0));
        assert!(dados.join("b/b.jpg").is_file() && !destino.join("b.jpg").exists());
        assert_eq!(iris.registro.borrow().len(), 2, "voltou para a Iris");
        let de_novo = apply(&plan_from_iris(&[("x".into(), "x.jpg".into())], &destino), &allowed, "Iris", Some(&iris)).unwrap();
        assert_eq!(de_novo.falhas.len(), 1, "arquivo que não está na Iris falha sem mexer em nada");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn pasta_e_programa_pelo_nome_dito() {
        let pastas = vec![PathBuf::from("/home/x/Downloads"), PathBuf::from("/home/x/Documents"), PathBuf::from("/home/x/Desktop")];
        assert_eq!(pasta_pelo_nome("documentos", &pastas), Some(PathBuf::from("/home/x/Documents")));
        assert_eq!(pasta_pelo_nome("area de trabalho", &pastas), Some(PathBuf::from("/home/x/Desktop")));
        assert_eq!(pasta_pelo_nome("musicas", &pastas), None, "não liberada");
        let programas = vec![
            Programa { id: "org.mozilla.firefox".into(), name: "Firefox".into(), comment: String::new(), icon: String::new() },
            Programa { id: "org.gnome.Nautilus".into(), name: "Arquivos".into(), comment: String::new(), icon: String::new() },
        ];
        assert_eq!(programa_pelo_nome("firefox", &programas).map(|p| p.id.as_str()), Some("org.mozilla.firefox"));
        assert_eq!(programa_pelo_nome("arquivos", &programas).map(|p| p.id.as_str()), Some("org.gnome.Nautilus"));
        assert_eq!(programa_pelo_nome("photoshop", &programas), None);
    }
}
