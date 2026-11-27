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
    pub tipo: Option<Tipo>,
    /// Só alterados nos últimos N dias.
    pub dias: Option<u32>,
    pub limit: usize,
}

/// Procura pelo nome (todas as palavras, sem diferenciar acentos) dentro de `root`, fora de pastas
/// ocultas e de `exclude`. Usa o índice do `plocate` quando existe e uma varredura limitada das
/// pastas do dia a dia (para achar o que é mais novo que o índice).
pub fn search(busca: &Busca, root: &Path, exclude: &[PathBuf], fresh: &[PathBuf]) -> Vec<Achado> {
    let words: Vec<String> = fold(busca.query).split_whitespace().map(str::to_string).collect();
    if words.is_empty() && busca.tipo.is_none() && busca.dias.is_none() { return Vec::new(); }
    let since = busca.dias.map(|d| now_secs() - i64::from(d) * 86_400);
    let accept = |path: &Path| -> Option<Achado> {
        let rest = path.strip_prefix(root).ok()?;
        if hidden(rest) || exclude.iter().any(|e| path.starts_with(e)) { return None; }
        if rest.components().any(|c| SKIP.iter().any(|s| c.as_os_str() == *s)) { return None; }
        let name = path.file_name()?.to_string_lossy().into_owned();
        let folded = fold(&name);
        if !words.iter().all(|w| folded.contains(w.as_str())) { return None; }
        let meta = fs::metadata(path).ok()?;
        let folder = meta.is_dir();
        match busca.tipo {
            Some(Tipo::Pasta) if !folder => return None,
            Some(tipo) if tipo != Tipo::Pasta && (folder || tipo_de(path) != Some(tipo)) => return None,
            _ => {}
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
            if text.starts_with('.') || SKIP.contains(&text.as_ref()) { continue; }
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
        let mut target = free_name(&to);
        if taken.contains(&target) {
            let stem = to.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let ext = to.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
            let dir = to.parent().unwrap_or(Path::new("/"));
            target = (2..).map(|n| dir.join(format!("{stem} {n}{ext}"))).find(|p| !p.exists() && !taken.contains(p)).unwrap_or(target);
        }
        taken.insert(target.clone());
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

fn move_path(from: &Path, to: &Path) -> std::io::Result<()> {
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
pub fn apply(plan: &[Passo], allowed: &[PathBuf], titulo: &str) -> Result<Resultado, String> {
    check(plan, allowed)?;
    let mut feitos = Vec::new();
    let mut falhas = Vec::new();
    for step in plan {
        let done = match step {
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
pub fn undo(lote: &Lote) -> (usize, Vec<String>) {
    let mut back = 0;
    let mut problems = Vec::new();
    for step in lote.feitos.iter().rev() {
        match step {
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

fn application_dirs() -> Vec<PathBuf> {
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
    let (mut name, mut name_pt, mut comment, mut comment_pt, mut icon) = (None, None, None, None, String::new());
    let (mut app, mut shown) = (false, true);
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') { main = line == "[Desktop Entry]"; continue; }
        if !main { continue; }
        let Some((key, value)) = line.split_once('=') else { continue };
        match key.trim() {
            "Type" => app = value.trim() == "Application",
            "NoDisplay" | "Hidden" if value.trim() == "true" => shown = false,
            "Name" => name = Some(value.trim().to_string()),
            "Name[pt_BR]" | "Name[pt]" => { name_pt.get_or_insert(value.trim().to_string()); }
            "Comment" => comment = Some(value.trim().to_string()),
            "Comment[pt_BR]" | "Comment[pt]" => { comment_pt.get_or_insert(value.trim().to_string()); }
            "Icon" => icon = value.trim().to_string(),
            _ => {}
        }
    }
    (app && shown).then_some(())?;
    Some(Programa { id: id.to_string(), name: name_pt.or(name)?, comment: comment_pt.or(comment).unwrap_or_default(), icon })
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
        let feito = apply(&plan, &allowed, "Downloads por tipo").unwrap();
        assert!(feito.falhas.is_empty());
        assert!(downloads.join("Imagens/foto.png").is_file() && downloads.join("Documentos/notas.txt").is_file());
        assert_eq!(fs::read_to_string(downloads.join("PDFs/contrato.pdf")).unwrap(), "antigo");
        let (back, problems) = undo(&feito.lote);
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
        let busca = |q: &str, tipo: Option<Tipo>| search(&Busca { query: q, tipo, dias: None, limit: 20 }, &root, &[], &[root.clone()]).into_iter().map(|a| a.name).collect::<Vec<_>>();
        let mut nomes = busca("relatorio", None); nomes.sort();
        assert_eq!(nomes, vec!["Relatório Anual.pdf", "relatorio rascunho.txt"]);
        assert_eq!(busca("relatório", Some(Tipo::Pdf)), vec!["Relatório Anual.pdf"]);
        assert_eq!(busca("docs", Some(Tipo::Pasta)), vec!["Docs"]);
        assert!(busca("", None).is_empty(), "sem pedido, sem lista");
        let fora = search(&Busca { query: "relatorio", tipo: None, dias: None, limit: 20 }, &root, &[root.join("Docs")], &[root.clone()]);
        assert!(fora.is_empty(), "pasta excluída (dados da Ayla) não aparece");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn programas_pelo_desktop_em_portugues_e_sem_ocultos() {
        let text = "[Desktop Entry]\nType=Application\nName=Files\nName[pt_BR]=Arquivos\nComment=Browse\nIcon=org.gnome.Nautilus\n[Desktop Action new]\nName=Nova janela\n";
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
