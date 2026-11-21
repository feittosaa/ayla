//! Athena: notas pessoais em Markdown, parte da própria Ayla.
//!
//! As notas são arquivos `.md` numa pasta (a do Obsidian, se a pessoa escolher, ou
//! `<dados da Ayla>/athena`). A Ayla só lê, grava e indexa: editar no Obsidian ou aqui dá no
//! mesmo. Apagar move para `.trash/` dentro da pasta, como o Obsidian. As notas nunca vão para
//! a nuvem.
use serde::Serialize;
use std::{
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

/// Bloco das notas no contexto da resposta local.
pub fn context_block(chunks: &[(String, String)]) -> Option<String> {
    if chunks.is_empty() {
        return None;
    }
    let body = chunks.iter().map(|(title, text)| format!("[nota: {title}]\n{}", text.trim())).collect::<Vec<_>>().join("\n\n");
    Some(format!(
        "\n\nTrechos das notas pessoais da Athena. Use-os quando ajudarem e cite a nota entre colchetes, como [nota: {}]. Se a resposta não estiver nas notas, não invente.\n\n{body}",
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
