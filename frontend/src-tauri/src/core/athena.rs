//! Athena: notas pessoais em Markdown, parte da própria Ayla.
//!
//! As notas são arquivos `.md` numa pasta (a do Obsidian, se a pessoa escolher, ou
//! `<dados da Ayla>/athena`). A Ayla só lê, grava e indexa: editar no Obsidian ou aqui dá no
//! mesmo. Apagar move para `.trash/` dentro da pasta, como o Obsidian. As notas nunca vão para
//! a nuvem.
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};

/// Limites para pastas grandes: notas listadas e tamanho lido de cada uma.
pub const MAX_NOTES: usize = 5_000;
pub const MAX_NOTE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Note {
    /// Caminho relativo à pasta, com `/` (ex.: "Projetos/Ideias.md").
    pub path: String,
    /// Título: nome do arquivo sem `.md`.
    pub title: String,
    /// Última modificação, em segundos Unix.
    pub modified: i64,
    pub bytes: u64,
}

/// Caminho relativo seguro de uma nota: só `.md`, sem `..`, sem absolutos e fora de pastas ocultas.
pub fn safe_relative(relative: &str) -> Option<PathBuf> {
    let path = Path::new(relative);
    let ok = !relative.is_empty()
        && relative.len() <= 1024
        && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("md"))
        && path.components().all(|c| matches!(c, Component::Normal(part) if !part.to_string_lossy().starts_with('.')));
    ok.then(|| path.to_path_buf())
}

fn modified(meta: &fs::Metadata) -> i64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn relative_string(path: &Path) -> String {
    path.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// Notas da pasta (recursivo), das mais recentes às mais antigas. Ignora pastas ocultas
/// (`.obsidian`, `.trash`) e não segue links simbólicos.
pub fn list(root: &Path) -> Vec<Note> {
    let mut out = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(root.join(&dir)) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let Ok(kind) = entry.file_type() else { continue };
            let relative = dir.join(&name);
            if kind.is_dir() {
                pending.push(relative);
            } else if kind.is_file() && safe_relative(&relative_string(&relative)).is_some() {
                let Ok(meta) = entry.metadata() else { continue };
                out.push(Note {
                    path: relative_string(&relative),
                    title: name.trim_end_matches(".md").trim_end_matches(".MD").to_string(),
                    modified: modified(&meta),
                    bytes: meta.len(),
                });
                if out.len() >= MAX_NOTES {
                    pending.clear();
                    break;
                }
            }
        }
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.path.cmp(&b.path)));
    out
}

/// Anexos que a Athena mostra (imagens e PDF), para a árvore, como o explorador do Obsidian.
pub fn attachments(root: &Path) -> Vec<Note> {
    scan_attachments(root, Some(MAX_NOTES)).0
}

pub fn scan_attachments(root: &Path, limit: Option<usize>) -> (Vec<Note>, Vec<String>) {
    let mut folders = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(root.join(&dir)) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let Ok(kind) = entry.file_type() else { continue };
            let relative = dir.join(&name);
            if kind.is_dir() {
                pending.push(relative);
            } else if kind.is_file() && attachment_mime(&relative).is_some() {
                let Ok(meta) = entry.metadata() else { continue };
                if !dir.as_os_str().is_empty() && folders.insert(relative_string(&dir)) {
                    for parent in dir.ancestors().skip(1).take_while(|parent| !parent.as_os_str().is_empty()) {
                        if !folders.insert(relative_string(parent)) { break; }
                    }
                }
                out.push(Note { path: relative_string(&relative), title: name, modified: modified(&meta), bytes: meta.len() });
                if limit.is_some_and(|max| out.len() >= max) {
                    pending.clear();
                    break;
                }
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    (out, folders.into_iter().collect())
}

/// Move um anexo para outra pasta da mesma raiz (sem sobrescrever). Links pelo nome continuam
/// valendo, porque os anexos são procurados pelo nome.
pub fn move_attachment(root: &Path, from: &str, folder: &str) -> Result<String, String> {
    let from_path = safe_folder(from).filter(|p| attachment_mime(p).is_some()).ok_or("Arquivo inválido")?;
    let name = from_path.file_name().ok_or("Arquivo inválido")?;
    let to_path = safe_folder(folder).ok_or("Pasta inválida")?.join(name);
    if !root.join(&from_path).is_file() {
        return Err("Arquivo não encontrado".into());
    }
    if root.join(&to_path).exists() {
        return Err("Já existe um arquivo com esse nome nessa pasta".into());
    }
    if let Some(parent) = root.join(&to_path).parent() {
        fs::create_dir_all(parent).map_err(|_| "Não foi possível criar a pasta")?;
    }
    fs::rename(root.join(&from_path), root.join(&to_path)).map_err(|_| "Não foi possível mover o arquivo")?;
    Ok(relative_string(&to_path))
}

/// Pastas (relativas, com `/`) para a árvore, inclusive as vazias; ignora as ocultas.
pub fn folders(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(root.join(&dir)) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || !entry.file_type().is_ok_and(|k| k.is_dir()) {
                continue;
            }
            let relative = dir.join(&name);
            out.push(relative_string(&relative));
            if out.len() >= MAX_NOTES {
                return out;
            }
            pending.push(relative);
        }
    }
    out.sort();
    out
}

/// Pasta relativa segura (vazia = raiz).
pub fn safe_folder(relative: &str) -> Option<PathBuf> {
    let path = Path::new(relative);
    let ok = relative.len() <= 1024
        && path.components().all(|c| matches!(c, Component::Normal(part) if !part.to_string_lossy().starts_with('.')));
    ok.then(|| path.to_path_buf())
}

/// Cria uma pasta dentro da raiz; devolve o caminho relativo.
pub fn mkdir(root: &Path, parent: &str, name: &str) -> Result<String, String> {
    let parent = safe_folder(parent).ok_or("Pasta inválida")?;
    let relative = parent.join(file_name(name));
    fs::create_dir_all(root.join(&relative)).map_err(|_| "Não foi possível criar a pasta")?;
    Ok(relative_string(&relative))
}

/// Conteúdo e data de modificação de uma nota.
pub fn read(root: &Path, relative: &str) -> Result<(String, i64), String> {
    let path = root.join(safe_relative(relative).ok_or("Nota inválida")?);
    let meta = fs::metadata(&path).map_err(|_| "Nota não encontrada")?;
    if meta.len() > MAX_NOTE_BYTES {
        return Err("Esta nota é grande demais para abrir aqui".into());
    }
    let text = fs::read_to_string(&path).map_err(|_| "Não foi possível ler a nota (ela precisa estar em UTF-8)")?;
    Ok((text, modified(&meta)))
}

/// Grava uma nota. Com `expected`, recusa se o arquivo mudou por fora (ex.: no Obsidian) desde
/// que foi aberto. Devolve a nova data de modificação.
pub fn write(root: &Path, relative: &str, content: &str, expected: Option<i64>) -> Result<i64, String> {
    let path = root.join(safe_relative(relative).ok_or("Nota inválida")?);
    if content.len() as u64 > MAX_NOTE_BYTES {
        return Err("Nota grande demais".into());
    }
    if let (Some(expected), Ok(meta)) = (expected, fs::metadata(&path)) {
        if modified(&meta) > expected {
            return Err("Esta nota mudou fora da Ayla. Ela foi recarregada; refaça a última alteração.".into());
        }
    }
    let dir = path.parent().ok_or("Nota inválida")?;
    fs::create_dir_all(dir).map_err(|_| "Não foi possível criar a pasta da nota")?;
    let temporary = dir.join(format!(".{}.ayla-tmp", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
    let result = (|| -> std::io::Result<()> {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
        return Err("Não foi possível salvar a nota".into());
    }
    fs::metadata(&path).map(|m| modified(&m)).map_err(|_| "Não foi possível salvar a nota".into())
}

/// Nome de arquivo a partir de um título (sem caracteres que o Obsidian ou o sistema recusam).
pub fn file_name(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '#' | '^' | '[' | ']') || c.is_control() { ' ' } else { c })
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned: String = cleaned.trim_start_matches('.').chars().take(120).collect();
    if cleaned.trim().is_empty() { "Nota sem título".into() } else { cleaned.trim().to_string() }
}

/// Cria uma nota nova sem sobrescrever outra ("Título 2.md", "Título 3.md"…).
pub fn create(root: &Path, title: &str, content: &str) -> Result<String, String> {
    create_in(root, "", title, content)
}
/// Cria a nota dentro de uma pasta da raiz.
pub fn create_in(root: &Path, folder: &str, title: &str, content: &str) -> Result<String, String> {
    let folder = safe_folder(folder).ok_or("Pasta inválida")?;
    let base = file_name(title);
    let mut relative = folder.join(format!("{base}.md"));
    let mut n = 2;
    while root.join(&relative).exists() {
        relative = folder.join(format!("{base} {n}.md"));
        n += 1;
    }
    let relative = relative_string(&relative);
    write(root, &relative, content, None)?;
    Ok(relative)
}

/// Links wiki de uma nota: alvos de `[[alvo]]`, `[[alvo|apelido]]` e `[[alvo#seção]]`.
pub fn wiki_targets(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else { break };
        let inner = &after[..end];
        let target = inner.split(['|', '#']).next().unwrap_or("").trim();
        if !target.is_empty() && !inner.contains('\n') {
            out.push(target.to_string());
        }
        rest = &after[end + 2..];
    }
    out
}

/// Alvos de links entre notas, ignorando exemplos de código.
pub fn graph_targets(text: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut fenced = false;
    for (code, part) in code_spans(text) {
        if code { continue; }
        for line in part.lines() {
            if line.trim_start().starts_with("~~~") {
                fenced = !fenced;
                continue;
            }
            if fenced { continue; }
            targets.extend(wiki_targets(line));
            for (start, _) in line.match_indices("](") {
                let tail = &line[start + 2..];
                let raw = if let Some(angle) = tail.strip_prefix('<') {
                    angle.split_once('>').map(|(url, _)| url)
                } else {
                    tail.split_once(')').map(|(url, _)| url.split_ascii_whitespace().next().unwrap_or(""))
                };
                let Some(url) = raw.and_then(|url| percent_decode(url.split('#').next().unwrap_or(""))) else { continue };
                if url.to_ascii_lowercase().ends_with(".md") && !url.starts_with('/') && !url.contains(':') {
                    targets.push(url.trim_start_matches("./").to_string());
                }
            }
        }
    }
    targets
}

/// O alvo de um link aponta para esta nota? Pelo nome (como o Obsidian) ou pelo caminho sem `.md`.
pub fn links_to(target: &str, note_path: &str) -> bool {
    let target = target.trim().trim_end_matches(".md").to_lowercase();
    let path = note_path.trim_end_matches(".md").to_lowercase();
    let name = path.rsplit('/').next().unwrap_or(&path);
    target == name || target == path || path.ends_with(&format!("/{target}"))
}

/// Move a nota para `.trash/` (lixeira do Obsidian) em vez de apagar.
pub fn trash(root: &Path, relative: &str) -> Result<(), String> {
    let relative = safe_relative(relative).ok_or("Nota inválida")?;
    let source = root.join(&relative);
    if !source.is_file() {
        return Err("Nota não encontrada".into());
    }
    let bin = root.join(".trash");
    fs::create_dir_all(&bin).map_err(|_| "Não foi possível criar a lixeira")?;
    let stem = relative.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let mut target = bin.join(format!("{stem}.md"));
    let mut n = 2;
    while target.exists() {
        target = bin.join(format!("{stem} {n}.md"));
        n += 1;
    }
    fs::rename(&source, &target).map_err(|_| "Não foi possível mover a nota para a lixeira".into())
}

/// Pasta inteira para a lixeira da raiz (`.trash/<nome>`), com tudo o que tiver dentro.
pub fn trash_folder(root: &Path, relative: &str) -> Result<(), String> {
    let relative = safe_folder(relative).filter(|p| !p.as_os_str().is_empty()).ok_or("Pasta inválida")?;
    let source = root.join(&relative);
    if !source.is_dir() {
        return Err("Pasta não encontrada".into());
    }
    let bin = root.join(".trash");
    fs::create_dir_all(&bin).map_err(|_| "Não foi possível criar a lixeira")?;
    let name = relative.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let mut target = bin.join(&name);
    let mut n = 2;
    while target.exists() {
        target = bin.join(format!("{name} {n}"));
        n += 1;
    }
    fs::rename(&source, &target).map_err(|_| "Não foi possível mover a pasta para a lixeira".into())
}

/// Nota do dia, no formato das notas diárias do Obsidian (`AAAA-MM-DD.md` na raiz).
pub fn daily(root: &Path, today: chrono::NaiveDate) -> Result<String, String> {
    let name = format!("{}.md", today.format("%Y-%m-%d"));
    if !root.join(&name).exists() {
        write(root, &name, &format!("# {}\n\n", today.format("%d/%m/%Y")), None)?;
    }
    Ok(name)
}

/// Cofres do Obsidian conhecidos (instalação comum e Flatpak), para sugerir em Configurações.
pub fn obsidian_vaults() -> Vec<String> {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    let mut out = Vec::new();
    for file in [config.join("obsidian/obsidian.json"), home.join(".var/app/md.obsidian.Obsidian/config/obsidian/obsidian.json")] {
        let Ok(text) = fs::read_to_string(file) else { continue };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        for vault in value["vaults"].as_object().into_iter().flat_map(|v| v.values()) {
            if let Some(path) = vault["path"].as_str() {
                if Path::new(path).is_dir() && !out.iter().any(|p| p == path) {
                    out.push(path.to_string());
                }
            }
        }
    }
    out
}

// Notas pela conversa: "cria uma nota sobre X: conteúdo", "salva isso como nota".

const CREATE: &[&str] = &[
    "cria uma nota", "crie uma nota", "criar uma nota", "cria nota", "crie nota", "criar nota", "nova nota",
    "faz uma nota", "faça uma nota", "escreve uma nota", "escreva uma nota",
];
const SAVE_LAST: &[&str] = &[
    "salva isso como nota", "salve isso como nota", "salva como nota", "salve como nota", "salva essa resposta como nota",
    "salve essa resposta como nota", "guarda isso como nota", "guarde isso como nota", "guarda como nota", "guarde como nota",
    "salva isso na athena", "salve isso na athena", "salva na athena", "salve na athena",
];

pub fn note_request(message: &str) -> bool {
    let text = message.to_lowercase();
    CREATE.iter().chain(SAVE_LAST).any(|t| text.contains(t))
}
pub fn save_last_request(message: &str) -> bool {
    let text = message.to_lowercase();
    SAVE_LAST.iter().any(|t| text.contains(t))
}
/// Palavras que fazem a interface tratar o pedido como local.
pub fn words() -> Vec<&'static str> {
    CREATE.iter().chain(SAVE_LAST).copied().collect()
}

/// Nota escrita no próprio pedido: "cria uma nota sobre Ideias de presente: livro, caneca".
/// Devolve (título, conteúdo) quando há conteúdo depois de ":".
pub fn explicit_note(message: &str) -> Option<(String, String)> {
    let (head, body) = message.split_once(':')?;
    let body = body.trim();
    if body.is_empty() {
        return None;
    }
    Some((title_from(head), body.to_string()))
}

/// Título a partir do pedido: o que vem depois de "sobre", "chamada", "com o título"…
pub fn title_from(request: &str) -> String {
    let lower = request.to_lowercase();
    for marker in ["com o título", "com titulo", "com título", "chamada", "intitulada", "sobre", "de", "para"] {
        let pattern = format!(" {marker} ");
        if let Some(at) = lower.find(&pattern) {
            let start = at + pattern.len();
            if let Some(rest) = request.get(start..) {
                let rest = rest.trim().trim_matches(|c: char| c == '"' || c == '“' || c == '”' || c == '\'');
                if !rest.is_empty() {
                    return file_name(&upper_first(rest));
                }
            }
        }
    }
    "Nota".into()
}

fn upper_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Pedido ao modelo local para escrever a nota a partir do pedido e da conversa recente.
pub fn model_request(message: &str, recent: &str) -> Vec<serde_json::Value> {
    let system = "Você escreve notas pessoais em Markdown para a pessoa guardar. Devolva só JSON com titulo (curto, sem a palavra nota) e conteudo (Markdown, em português, sem inventar fatos que não estejam no pedido ou na conversa).";
    vec![
        serde_json::json!({"role":"system","content":system}),
        serde_json::json!({"role":"user","content":format!("Conversa recente:\n{recent}\n\nPedido: {message}")}),
    ]
}
pub fn model_schema() -> serde_json::Value {
    serde_json::json!({"type":"object","properties":{
        "titulo":{"type":"string","minLength":1,"maxLength":120},
        "conteudo":{"type":"string","minLength":1,"maxLength":20000}
    },"required":["titulo","conteudo"]})
}
pub fn parse_model(raw: &str) -> Option<(String, String)> {
    let value: serde_json::Value = serde_json::from_str(raw.trim()).ok()?;
    let title = value["titulo"].as_str()?.trim();
    let content = value["conteudo"].as_str()?.trim();
    (!title.is_empty() && !content.is_empty()).then(|| (file_name(title), content.to_string()))
}

// Renomear e mover, com os links atualizados (como o Obsidian).

/// Troca, no texto, os links wiki que apontam para `old` (pelo nome ou caminho) por `target`,
/// mantendo seção e apelido; também links Markdown para o caminho antigo. Devolve o texto novo,
/// se mudou.
pub fn relink(text: &str, old: &str, target: &str, new_path: &str) -> Option<String> {
    // Código (em bloco ou em linha) fica como está, como no Obsidian.
    let mut out = String::new();
    let mut changed = false;
    for (code, part) in code_spans(text) {
        match (code, relink_plain(part, old, target, new_path)) {
            (false, Some(updated)) => {
                out.push_str(&updated);
                changed = true;
            }
            _ => out.push_str(part),
        }
    }
    changed.then_some(out)
}

/// Divide o texto em trechos de código (```bloco``` e `linha`) e o resto.
fn code_spans(text: &str) -> Vec<(bool, &str)> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let fence = rest.find("```");
        let tick = rest.find('`');
        let Some(start) = fence.or(tick).map(|f| tick.map_or(f, |t| t.min(f))) else {
            out.push((false, rest));
            break;
        };
        let (marker, len) = if fence == Some(start) { ("```", 3) } else { ("`", 1) };
        let Some(end) = rest[start + len..].find(marker).map(|e| start + len + e + len) else {
            out.push((false, rest));
            break;
        };
        if start > 0 {
            out.push((false, &rest[..start]));
        }
        out.push((true, &rest[start..end]));
        rest = &rest[end..];
    }
    out
}

fn relink_plain(text: &str, old: &str, target: &str, new_path: &str) -> Option<String> {
    let mut out = String::new();
    let mut rest = text;
    let mut changed = false;
    while let Some(start) = rest.find("[[") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let inner = &after[..end];
        let cut = inner.find(['|', '#']).unwrap_or(inner.len());
        let (name, tail) = inner.split_at(cut);
        if !inner.contains('\n') && links_to(name, old) {
            out.push_str(&format!("[[{target}{tail}]]"));
            changed = true;
        } else {
            out.push_str(&format!("[[{inner}]]"));
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    for (from, to) in [(old.to_string(), new_path.to_string()), (old.replace(' ', "%20"), new_path.replace(' ', "%20"))] {
        let pattern = format!("]({from})");
        if out.contains(&pattern) {
            out = out.replace(&pattern, &format!("]({to})"));
            changed = true;
        }
    }
    changed.then_some(out)
}

/// Como os links devem citar a nota: pelo nome, se ele for único na raiz; senão, pelo caminho.
pub fn link_target(notes: &[Note], path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path).trim_end_matches(".md");
    let same = notes.iter().filter(|n| n.title.eq_ignore_ascii_case(name)).count();
    if same <= 1 { name.to_string() } else { path.trim_end_matches(".md").to_string() }
}

/// Atualiza os links de todas as notas da raiz para as mudanças (antigo, novo). Devolve quantas notas mudaram.
fn relink_all(root: &Path, moves: &[(String, String)]) -> usize {
    let notes = list(root);
    let mut changed = 0;
    for note in &notes {
        let Ok((mut text, _)) = read(root, &note.path) else { continue };
        let mut touched = false;
        for (old, new) in moves {
            if let Some(updated) = relink(&text, old, &link_target(&notes, new), new) {
                text = updated;
                touched = true;
            }
        }
        if touched && write(root, &note.path, &text, None).is_ok() {
            changed += 1;
        }
    }
    changed
}

/// Renomeia ou move uma nota dentro da raiz (pastas criadas se preciso) e atualiza os links.
/// Devolve o caminho novo e quantas notas tiveram links atualizados.
pub fn rename_note(root: &Path, from: &str, to: &str) -> Result<(String, usize), String> {
    let from_path = safe_relative(from).ok_or("Nota inválida")?;
    let to = if to.to_lowercase().ends_with(".md") { to.to_string() } else { format!("{to}.md") };
    let to_path = safe_relative(&to).ok_or("Nome ou pasta inválidos")?;
    let name = to_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    if file_name(&name) != name {
        return Err("O nome não pode ter / \\ : * ? \" < > | # ^ [ ]".into());
    }
    if !root.join(&from_path).is_file() {
        return Err("Nota não encontrada".into());
    }
    if root.join(&to_path).exists() {
        return Err("Já existe uma nota com esse nome nessa pasta".into());
    }
    if let Some(parent) = root.join(&to_path).parent() {
        fs::create_dir_all(parent).map_err(|_| "Não foi possível criar a pasta")?;
    }
    fs::rename(root.join(&from_path), root.join(&to_path)).map_err(|_| "Não foi possível renomear a nota")?;
    let (old, new) = (relative_string(&from_path), relative_string(&to_path));
    let changed = relink_all(root, &[(old, new.clone())]);
    Ok((new, changed))
}

/// Renomeia ou move uma pasta inteira e atualiza os links que usavam o caminho das notas dela.
pub fn rename_folder(root: &Path, from: &str, to: &str) -> Result<(String, usize), String> {
    let from_path = safe_folder(from).filter(|p| !p.as_os_str().is_empty()).ok_or("Pasta inválida")?;
    let to_path = safe_folder(to).filter(|p| !p.as_os_str().is_empty()).ok_or("Nome inválido")?;
    if to_path.starts_with(&from_path) {
        return Err("Uma pasta não pode ir para dentro dela mesma".into());
    }
    if !root.join(&from_path).is_dir() {
        return Err("Pasta não encontrada".into());
    }
    if root.join(&to_path).exists() {
        return Err("Já existe uma pasta com esse nome".into());
    }
    let (old, new) = (relative_string(&from_path), relative_string(&to_path));
    let moved: Vec<(String, String)> = list(root)
        .into_iter()
        .filter(|n| n.path.starts_with(&format!("{old}/")))
        .map(|n| (n.path.clone(), format!("{new}{}", &n.path[old.len()..])))
        .collect();
    if let Some(parent) = root.join(&to_path).parent() {
        fs::create_dir_all(parent).map_err(|_| "Não foi possível criar a pasta")?;
    }
    fs::rename(root.join(&from_path), root.join(&to_path)).map_err(|_| "Não foi possível renomear a pasta")?;
    let changed = relink_all(root, &moved);
    Ok((new, changed))
}

// Tags (#tag no texto e `tags:` no cabeçalho YAML, como o Obsidian).

/// Tags de uma nota, sem `#`, em minúsculas, sem repetição. Ignora código, títulos e âncoras de links.
pub fn tags(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |tag: &str| {
        let tag = tag.trim().trim_start_matches('#').trim_matches(|c: char| c == '"' || c == '\'').to_lowercase();
        if !tag.is_empty() && tag.chars().any(|c| !c.is_ascii_digit()) && !out.contains(&tag) {
            out.push(tag);
        }
    };
    let mut body = text;
    // Cabeçalho YAML: "tags: [a, b]", "tags: a, b" ou lista "- a".
    if let Some(rest) = text.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---") {
            let yaml = &rest[..end];
            let mut in_tags = false;
            for line in yaml.lines() {
                let trimmed = line.trim();
                if let Some(value) = trimmed.strip_prefix("tags:").or_else(|| trimmed.strip_prefix("tag:")) {
                    in_tags = value.trim().is_empty();
                    for tag in value.trim().trim_matches(['[', ']']).split(',') {
                        push(tag);
                    }
                } else if in_tags && trimmed.starts_with("- ") {
                    push(&trimmed[2..]);
                } else if !trimmed.is_empty() {
                    in_tags = false;
                }
            }
            body = &rest[end + 4..];
        }
    }
    let mut in_code = false;
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        let line: String = line.split('`').enumerate().filter(|(i, _)| i % 2 == 0).map(|(_, p)| p).collect::<Vec<_>>().join(" ");
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '#' && (i == 0 || chars[i - 1].is_whitespace()) {
                let tag: String = chars[i + 1..].iter().take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/')).collect();
                if !tag.is_empty() {
                    push(&tag);
                    i += tag.chars().count();
                }
            }
            i += 1;
        }
    }
    out
}

// Anexos (![[foto.png]]): procurados pelo nome na raiz, como o Obsidian.

pub const ATTACHMENT_IMAGES: &[&str] = &["png", "jpg", "jpeg", "jfif", "gif", "webp", "bmp", "svg", "avif"];
pub const MAX_ATTACHMENT: u64 = 15 * 1024 * 1024;

/// Tipo do anexo que a Athena e a Iris mostram (imagens, vídeos e PDF); outros arquivos não são servidos.
pub fn attachment_mime(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_string_lossy().to_lowercase();
    Some(match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" | "jfif" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "avif" => "image/avif",
        "pdf" => "application/pdf",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "ogv" => "video/ogg",
        "mkv" => "video/x-matroska",
        _ => return None,
    })
}

/// Guarda uma imagem colada e grava a referência na nota na mesma operação.
pub fn paste_image(root: &Path, note: &str, before: &str, after: &str, expected: i64, bytes: &[u8]) -> Result<(String, i64), String> {
    let (target, _, modified) = save_note_image(root, note, expected, bytes, |target| format!("{before}![[{target}]]{after}"))?;
    Ok((format!("![[{target}]]"), modified))
}

/// Usa como banner uma imagem que já está na mesma raiz da nota (cofre, projeto ou Athena).
pub fn set_banner(root: &Path, note: &str, content: &str, expected: i64, image: &str) -> Result<(String, i64), String> {
    let root = root.canonicalize().map_err(|_| "Pasta da nota indisponível")?;
    let note_path = root.join(safe_relative(note).ok_or("Nota inválida")?);
    if !note_path.is_file() {
        return Err("Nota não encontrada na pasta da Athena".into());
    }
    let image = image.trim().trim_start_matches("./");
    let relative = Path::new(image);
    let inside = !image.is_empty()
        && relative.components().all(|c| matches!(c, Component::Normal(part) if !part.to_string_lossy().starts_with('.')))
        && attachment_mime(relative).is_some_and(|mime| mime.starts_with("image/"));
    let found = root.join(relative).canonicalize().ok().filter(|path| inside && path.starts_with(&root) && path.is_file());
    if found.is_none() {
        return Err("Escolha uma imagem desta mesma pasta".into());
    }
    let updated = with_banner(content, &relative_string(relative));
    let modified = write(&root, note, &updated, Some(expected))?;
    Ok((updated, modified))
}

/// Altura dos banners no make-md do cofre ("Cover Height"; 200 px quando não configurada).
pub fn make_md_banner_height(root: &Path) -> Option<u32> {
    let file = root.join(".obsidian/plugins/make-md/data.json");
    let settings: serde_json::Value = serde_json::from_slice(&fs::read(file).ok()?).ok()?;
    let height = settings.get("bannerHeight").and_then(|h| h.as_f64().or_else(|| h.as_str()?.trim().parse().ok())).unwrap_or(200.0);
    (height.is_finite() && height > 0.0).then(|| height.round().clamp(80.0, 600.0) as u32)
}

/// Texto em YAML como o make-md grava (`Imagens/capa.png`, sem aspas); com aspas só quando o
/// valor seria lido de outro jeito.
fn yaml_text(value: &str) -> String {
    let plain = !value.is_empty()
        && value.trim() == value
        && !value.starts_with(|c: char| "-?:,[]{}#&*!|>'\"%@`".contains(c))
        && !value.contains(": ") && !value.contains(" #") && !value.ends_with(':')
        && !value.chars().any(char::is_control)
        && !matches!(value.to_ascii_lowercase().as_str(), "true" | "false" | "null" | "yes" | "no" | "~")
        && value.parse::<f64>().is_err();
    if plain { value.to_string() } else { format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")) }
}

pub fn with_banner(content: &str, target: &str) -> String {
    let newline = if content.contains("\r\n") { "\r\n" } else { "\n" };
    let banner = format!("banner: {}{newline}", yaml_text(target));
    let mut lines = content.split_inclusive('\n');
    if let Some(opening) = lines.next().filter(|line| line.trim_end() == "---") {
        let mut updated = String::with_capacity(content.len() + banner.len());
        updated.push_str(opening);
        let mut offset = opening.len();
        for line in lines {
            if line.trim_end() == "---" {
                updated.push_str(&banner);
                updated.push_str(&content[offset..]);
                return updated;
            }
            if !line.starts_with("banner:") { updated.push_str(line); }
            offset += line.len();
        }
    }
    format!("---{newline}{banner}---{newline}{newline}{content}")
}

/// Extensão de uma imagem colada, pelo conteúdo (não pelo nome).
pub fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    Some(match bytes {
        [137, 80, 78, 71, 13, 10, 26, 10, ..] => "png",
        [255, 216, 255, ..] => "jpg",
        [b'G', b'I', b'F', b'8', b'7' | b'9', b'a', ..] => "gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "webp",
        [b'B', b'M', ..] => "bmp",
        [_, _, _, _, b'f', b't', b'y', b'p', b'a', b'v', b'i', b'f' | b's', ..] => "avif",
        _ => return None,
    })
}

fn save_note_image(root: &Path, note: &str, expected: i64, bytes: &[u8], content: impl FnOnce(&str) -> String) -> Result<(String, String, i64), String> {
    if bytes.len() as u64 > MAX_ATTACHMENT {
        return Err("A imagem passa de 15 MB".into());
    }
    let extension = image_extension(bytes).ok_or("Formato de imagem incompatível")?;
    let relative = safe_relative(note).ok_or("Nota inválida")?;
    let root = root.canonicalize().map_err(|_| "Pasta da nota indisponível")?;
    let path = root.join(&relative);
    if path.canonicalize().ok().as_ref() != Some(&path) || !path.is_file() {
        return Err("Nota não encontrada na pasta da Athena".into());
    }
    let title = relative.file_stem().ok_or("Nota inválida")?;
    let images = root.join("Imagens");
    let folder = images.join(title);
    for dir in [&images, &folder] {
        match fs::create_dir(dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && fs::symlink_metadata(dir).is_ok_and(|meta| meta.is_dir()) => {}
            Err(_) => return Err("Não foi possível criar a pasta de imagens da nota".into()),
        }
    }

    let mut random = [0; 16];
    getrandom::fill(&mut random).map_err(|_| "Não foi possível gerar o nome da imagem")?;
    let id: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let name = format!("Imagem-{id}.{extension}");
    let image = folder.join(&name);
    let title = title.to_string_lossy();
    // Referências por caminho abrem direto. Títulos com sintaxe wiki usam o nome único.
    let target = if title.contains(['[', ']', '#', '|', '\n', '\r', '\\']) {
        name
    } else {
        format!("Imagens/{title}/{name}")
    };
    let mut file = fs::File::create_new(&image).map_err(|_| "Não foi possível criar a imagem")?;
    let written = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file);
    let content = content(&target);
    let result = written.map_err(|_| "Não foi possível salvar a imagem".to_string())
        .and_then(|_| write(&root, note, &content, Some(expected)));
    if result.is_err() {
        let _ = fs::remove_file(&image);
    }
    result.map(|modified| (target, content, modified))
}

/// Decodifica `%XX` de um trecho de URL (nomes com espaço e acentos).
pub fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Caminho de um anexo pelo nome (ou caminho relativo), dentro da raiz e fora de pastas ocultas.
pub fn find_attachment(root: &Path, target: &str) -> Option<PathBuf> {
    let direct = safe_folder(target).map(|p| root.join(p)).filter(|p| p.is_file());
    if direct.is_some() {
        return direct;
    }
    let wanted = target.rsplit('/').next()?.to_lowercase();
    let mut pending = vec![root.to_path_buf()];
    let mut seen = 0;
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            seen += 1;
            if seen > 50_000 {
                return None;
            }
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => pending.push(entry.path()),
                Ok(kind) if kind.is_file() && name.to_lowercase() == wanted => return Some(entry.path()),
                _ => {}
            }
        }
    }
    None
}

/// Pedidos em que vale consultar uma pasta como coleção de notas.
pub(super) fn collection_question(message: &str) -> bool {
    message.to_lowercase().split(|c: char| !c.is_alphanumeric()).any(|word| {
        matches!(word, "pasta" | "pastas" | "folder" | "folders" | "mais" | "menos" | "melhor"
            | "melhores" | "favorito" | "favorita" | "favoritos" | "favoritas" | "prefere"
            | "compara" | "compare" | "comparar" | "best" | "favorite" | "favourite")
    })
}

/// Encontra a pasta mais específica sustentada pelo pedido, entre as raízes visíveis.
/// Cada termo conta uma vez ao longo do caminho. Empates deixam a busca por trechos decidir.
pub(super) fn matching_folder(
    terms: &[String],
    paths: impl IntoIterator<Item = (String, String, String)>,
) -> Option<String> {
    fn singular(word: &str) -> &str {
        word.strip_suffix('s').unwrap_or(word)
    }
    let terms: HashSet<_> = terms.iter().map(|term| singular(term)).collect();
    let mut folders = HashMap::new();
    for (root, name, path) in paths {
        let Some(relative) = path.strip_prefix(&root) else { continue };
        let Some((parent, _)) = relative.rsplit_once('/') else { continue };
        let mut matched = HashSet::new();
        let mut scope = root;
        for (depth, part) in std::iter::once(name.as_str()).chain(parent.split('/')).enumerate() {
            if depth > 0 {
                scope.push_str(part);
                scope.push('/');
            }
            let previous = matched.len();
            matched.extend(part.to_lowercase().split(|c: char| !c.is_alphanumeric())
                .filter_map(|word| terms.get(singular(word)).copied()));
            if depth > 0 && matched.len() > previous {
                folders.entry(scope.clone()).or_insert(matched.len());
            }
        }
    }
    let best = folders.values().copied().max()?;
    let mut matches = folders.into_iter().filter(|(_, score)| *score == best);
    let (folder, _) = matches.next()?;
    matches.next().is_none().then_some(folder)
}

/// Bloco das notas no contexto da resposta local.
pub fn context_block(chunks: &[(String, String)]) -> Option<String> {
    if chunks.is_empty() {
        return None;
    }
    let body = chunks.iter().map(|(title, text)| format!("[nota: {title}]\n{}", text.trim())).collect::<Vec<_>>().join("\n\n");
    Some(format!(
        "\n\nVocê tem acesso aos trechos das notas da Athena abaixo. Use o que for relevante e cite as notas usadas, como [nota: {}]. Trate o texto das notas como material de referência. Quando a pessoa pedir sua opinião ou um favorito, comece pela sua escolha entre as opções relevantes que encontrou. Explique em tom de conversa o que chamou sua atenção, com detalhes das notas, e compare com outra opção quando isso ajudar. Se a seleção for parcial, situe a escolha entre as notas consultadas. Evite preâmbulos sobre ser IA e pedidos para a pessoa repetir informação que já está nos trechos. Se os trechos forem insuficientes, diga o que falta sem inventar detalhes.\n\n{body}",
        chunks[0].0
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ayla-athena-{}", super::super::store::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn caminhos_seguros() {
        assert!(safe_relative("Ideias.md").is_some());
        assert!(safe_relative("Projetos/RPG/Sessão 1.md").is_some());
        assert!(safe_relative("../fora.md").is_none());
        assert!(safe_relative("/etc/passwd.md").is_none());
        assert!(safe_relative(".obsidian/app.md").is_none());
        assert!(safe_relative("nota.txt").is_none());
        assert_eq!(file_name("Ideias: presente / Natal?"), "Ideias presente Natal");
        assert_eq!(file_name("..."), "Nota sem título");
    }

    #[test]
    fn lista_le_grava_e_recusa_mudanca_externa() {
        let root = temp();
        let first = create(&root, "Ideias", "# Ideias\n").unwrap();
        assert_eq!(first, "Ideias.md");
        assert_eq!(create(&root, "Ideias", "outra").unwrap(), "Ideias 2.md", "não sobrescreve");
        fs::create_dir_all(root.join(".obsidian")).unwrap();
        fs::write(root.join(".obsidian/config.md"), "x").unwrap();
        fs::create_dir_all(root.join("RPG")).unwrap();
        fs::write(root.join("RPG/Sessão 1.md"), "Mestre: eu").unwrap();
        fs::write(root.join("RPG/imagem.png"), "x").unwrap();
        let mut paths: Vec<String> = list(&root).into_iter().map(|n| n.path).collect();
        paths.sort();
        assert_eq!(paths, ["Ideias 2.md", "Ideias.md", "RPG/Sessão 1.md"]);
        let (text, stamp) = read(&root, "Ideias.md").unwrap();
        assert_eq!(text, "# Ideias\n");
        let stamp = write(&root, "Ideias.md", "# Ideias\n- livro\n", Some(stamp)).unwrap();
        assert!(write(&root, "Ideias.md", "velho", Some(stamp - 10)).is_err(), "mudou desde que abriu");
        assert!(write(&root, "../fora.md", "x", None).is_err());
        trash(&root, "Ideias.md").unwrap();
        assert!(!root.join("Ideias.md").exists() && root.join(".trash/Ideias.md").exists());
        assert!(list(&root).iter().all(|n| !n.path.contains(".trash")));
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        assert_eq!(daily(&root, today).unwrap(), "2026-11-21.md");
        assert_eq!(read(&root, "2026-11-21.md").unwrap().0, "# 21/11/2026\n\n");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn colar_imagem_grava_na_pasta_da_nota_e_preserva_o_texto() {
        let root = temp();
        let note = create_in(&root, "Sessões", "Mapa da região", "Antes depois").unwrap();
        let png = b"\x89PNG\r\n\x1a\nfixture";
        let (_, stamp) = read(&root, &note).unwrap();
        let (embed, stamp) = paste_image(&root, &note, "Antes ", " depois", stamp, png).unwrap();
        let target = &embed[3..embed.len() - 2];
        assert!(target.starts_with("Imagens/Mapa da região/"));
        assert_eq!(fs::read(root.join(target)).unwrap(), png);
        assert_eq!(read(&root, &note).unwrap().0, format!("Antes {embed} depois"));
        assert_eq!(find_attachment(&root, target), Some(root.join(target)));

        let (another, stamp) = paste_image(&root, &note, &embed, "", stamp, png).unwrap();
        assert_ne!(another, embed);
        assert!(paste_image(&root, &note, "fora", "", stamp - 10, png).is_err());
        assert!(paste_image(&root, &note, "fora", "", stamp, b"texto").is_err());
        assert_eq!(read(&root, &note).unwrap().0, format!("{embed}{another}"));
        assert_eq!(attachments(&root).len(), 2);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn banner_usa_imagem_da_mesma_raiz_sem_copiar() {
        let root = temp();
        let outside = temp();
        let source = "---\r\nbanner: \"\"\r\ntags: [mapa]\r\n---\r\nTexto com [[Link]].";
        let note = create_in(&root, "", "Nota", source).unwrap();
        fs::create_dir_all(root.join("Mapas")).unwrap();
        fs::write(root.join("Mapas/Norte.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        fs::write(root.join("texto.pdf"), b"%PDF").unwrap();
        fs::write(outside.join("fora.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        let (_, modified) = read(&root, &note).unwrap();
        for wrong in ["", "texto.pdf", "Ausente.png", "../fora.png", "/etc/hostname.png", ".obsidian/x.png"] {
            assert!(set_banner(&root, &note, source, modified, wrong).is_err(), "{wrong}");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.join("fora.png"), root.join("atalho.png")).unwrap();
            assert!(set_banner(&root, &note, source, modified, "atalho.png").is_err(), "link para fora da raiz");
        }
        assert_eq!(read(&root, &note).unwrap().0, source, "recusas não alteram a nota");
        let before = attachments(&root).len();
        let (updated, _) = set_banner(&root, &note, source, modified, "Mapas/Norte.png").unwrap();
        assert_eq!(updated, "---\r\ntags: [mapa]\r\nbanner: Mapas/Norte.png\r\n---\r\nTexto com [[Link]].", "mesmo formato do make-md");
        assert_eq!(read(&root, &note).unwrap().0, updated);
        assert_eq!(attachments(&root).len(), before, "nada foi copiado");
        assert_eq!(with_banner("Texto", "mapa.png"), "---\nbanner: mapa.png\n---\n\nTexto");
        assert_eq!(yaml_text("Imagens/Personagens/As 6 Faces/Tauri.png"), "Imagens/Personagens/As 6 Faces/Tauri.png");
        assert_eq!(yaml_text("Ganchos/G5/_358c1d1b.jpg"), "Ganchos/G5/_358c1d1b.jpg");
        assert_eq!(yaml_text("#capa: a.png"), "\"#capa: a.png\"");
        assert_eq!(yaml_text("[x] \"a\".png"), "\"[x] \\\"a\\\".png\"");
        assert_eq!(yaml_text("2024"), "\"2024\"");
        assert_eq!(make_md_banner_height(&root), None, "sem make-md");
        fs::create_dir_all(root.join(".obsidian/plugins/make-md")).unwrap();
        fs::write(root.join(".obsidian/plugins/make-md/data.json"), "{}").unwrap();
        assert_eq!(make_md_banner_height(&root), Some(200));
        fs::write(root.join(".obsidian/plugins/make-md/data.json"), r#"{"bannerHeight": "260"}"#).unwrap();
        assert_eq!(make_md_banner_height(&root), Some(260));
        assert!(set_banner(&root, &note, &updated, i64::MIN, "Mapas/Norte.png").is_err());
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    #[cfg(unix)]
    fn colar_imagem_recusa_pasta_com_link_simbolico() {
        let root = temp();
        let outside = temp();
        let note = create(&root, "Mapa", "").unwrap();
        let (_, stamp) = read(&root, &note).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("Imagens")).unwrap();
        assert!(paste_image(&root, &note, "", "", stamp, b"\x89PNG\r\n\x1a\nfixture").is_err());
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
        assert_eq!(read(&root, &note).unwrap().0, "");
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn pastas_e_links() {
        let root = temp();
        assert_eq!(mkdir(&root, "", "RPG").unwrap(), "RPG");
        assert_eq!(mkdir(&root, "RPG", "Sessões").unwrap(), "RPG/Sessões");
        assert!(mkdir(&root, "../fora", "x").is_err());
        assert_eq!(create_in(&root, "RPG/Sessões", "Sessão 1", "Ver [[Personagens|ficha]] e [[Mapa#Norte]].").unwrap(), "RPG/Sessões/Sessão 1.md");
        std::fs::create_dir_all(root.join(".obsidian")).unwrap();
        assert_eq!(folders(&root), ["RPG", "RPG/Sessões"]);
        assert_eq!(wiki_targets("Ver [[Personagens|ficha]] e [[Mapa#Norte]] e [[ ]]."), ["Personagens", "Mapa"]);
        assert!(links_to("Personagens", "RPG/Personagens.md"));
        assert!(links_to("rpg/personagens", "RPG/Personagens.md"));
        assert!(!links_to("Mapa", "RPG/Mapa antigo.md"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn renomear_e_mover_atualiza_links() {
        let root = temp();
        create_in(&root, "RPG", "Personagens", "# Personagens\n").unwrap();
        create_in(&root, "", "Índice", "Ver [[Personagens]], [[Personagens#Elfos|os elfos]] e [[RPG/Personagens]]. Também [ficha](RPG/Personagens.md). `[[Personagens]]` fica.").unwrap();
        let (path, changed) = rename_note(&root, "RPG/Personagens.md", "RPG/Elenco").unwrap();
        assert_eq!((path.as_str(), changed), ("RPG/Elenco.md", 1));
        let index = read(&root, "Índice.md").unwrap().0;
        assert_eq!(index, "Ver [[Elenco]], [[Elenco#Elfos|os elfos]] e [[Elenco]]. Também [ficha](RPG/Elenco.md). `[[Personagens]]` fica.");
        assert!(rename_note(&root, "RPG/Elenco.md", "Índice").is_err(), "não sobrescreve");
        assert!(rename_note(&root, "RPG/Elenco.md", "../fora").is_err());
        let (folder, _) = rename_folder(&root, "RPG", "Jogos/RPG").unwrap();
        assert_eq!(folder, "Jogos/RPG");
        assert!(root.join("Jogos/RPG/Elenco.md").is_file());
        assert!(rename_folder(&root, "Jogos", "Jogos/RPG/dentro").is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn lixeira_de_pasta_tipos_e_enderecos() {
        let root = temp();
        fs::create_dir_all(root.join("Jogos/RPG")).unwrap();
        fs::write(root.join("Jogos/RPG/Sessão.md"), "x").unwrap();
        fs::create_dir_all(root.join(".trash/RPG")).unwrap();
        assert!(trash_folder(&root, "").is_err() && trash_folder(&root, "../fora").is_err() && trash_folder(&root, "Nada").is_err());
        trash_folder(&root, "Jogos/RPG").unwrap();
        assert!(!root.join("Jogos/RPG").exists());
        assert!(root.join(".trash/RPG 2/Sessão.md").is_file(), "não sobrescreve o que já está na lixeira");
        fs::create_dir_all(root.join("Livros/.oculta")).unwrap();
        fs::write(root.join("Livros/Regras.pdf"), "%PDF").unwrap();
        fs::write(root.join("Livros/.oculta/x.pdf"), "%PDF").unwrap();
        fs::write(root.join("Livros/script.sh"), "x").unwrap();
        let found: Vec<String> = attachments(&root).into_iter().map(|n| n.path).collect();
        assert_eq!(found, vec!["Livros/Regras.pdf".to_string()]);
        fs::create_dir_all(root.join("Livros/Acervo/Vazio")).unwrap();
        let (media, folders) = scan_attachments(&root, Some(MAX_NOTES));
        assert_eq!(media.len(), 1);
        assert_eq!(folders, ["Livros"]);
        assert_eq!(move_attachment(&root, "Livros/Regras.pdf", "").unwrap(), "Regras.pdf");
        assert!(root.join("Regras.pdf").is_file());
        assert!(move_attachment(&root, "Livros/script.sh", "").is_err() && move_attachment(&root, "../x.pdf", "").is_err());
        assert_eq!(attachment_mime(Path::new("a/Foto.JFIF")), Some("image/jpeg"));
        assert_eq!(attachment_mime(Path::new("livro.pdf")), Some("application/pdf"));
        assert_eq!(attachment_mime(Path::new("script.sh")), None);
        assert_eq!(attachment_mime(Path::new("Sessão 1.MP4")), Some("video/mp4"));
        assert_eq!(percent_decode("Minha%20foto%C3%A9.png").as_deref(), Some("Minha fotoé.png"));
        assert_eq!(percent_decode("quebrado%2"), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn tags_e_anexos() {
        assert_eq!(tags("---\ntags: [treino, saúde]\n---\n# Título\nTexto #Academia e #rpg/sessão, não #1 nem a#b.\n```\n#codigo\n```"), ["treino", "saúde", "academia", "rpg/sessão"]);
        assert_eq!(tags("---\ntags:\n  - a\n  - b\n---\n"), ["a", "b"]);
        assert!(tags("## Título\n[link](x#ancora) `#codigo`").is_empty());
        let root = temp();
        fs::create_dir_all(root.join("anexos")).unwrap();
        fs::write(root.join("anexos/Foto 1.png"), "x").unwrap();
        assert_eq!(find_attachment(&root, "foto 1.png"), Some(root.join("anexos/Foto 1.png")));
        assert_eq!(find_attachment(&root, "anexos/Foto 1.png"), Some(root.join("anexos/Foto 1.png")));
        assert_eq!(find_attachment(&root, "../fora.png"), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn notas_pela_conversa() {
        assert!(note_request("cria uma nota sobre ideias de presente: livro, caneca"));
        assert!(note_request("Salva isso como nota"));
        assert!(save_last_request("salva essa resposta como nota"));
        assert!(!note_request("anote que eu gosto de café"), "isso é memória");
        assert_eq!(explicit_note("cria uma nota sobre ideias de presente: livro, caneca"), Some(("Ideias de presente".into(), "livro, caneca".into())));
        assert_eq!(explicit_note("cria uma nota sobre a reunião"), None);
        assert_eq!(title_from("cria uma nota chamada \"Plano de treino\""), "Plano de treino");
        assert_eq!(title_from("cria uma nota"), "Nota");
        assert_eq!(parse_model(r#"{"titulo":"Treino: semana 1","conteudo":"- agachamento"}"#), Some(("Treino semana 1".into(), "- agachamento".into())));
        assert!(context_block(&[("Ideias".into(), "livro".into())]).unwrap().contains("[nota: Ideias]\nlivro"));
    }
}
