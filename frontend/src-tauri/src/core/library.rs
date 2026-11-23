//! Biblioteca da Athena: livros (EPUB) e mangás (CBZ) lidos no próprio app.
//!
//! Fontes: a pasta `biblioteca` da Ayla (arquivos importados, copiados) e pastas conectadas (ex.:
//! a biblioteca do MAD), lidas no lugar, sem cópia. Uma série é a pasta de um livro; os capítulos
//! seguem a ordem natural dos nomes (Capitulo_9 antes de Capitulo_10).
//!
//! O progresso fica em `leitura.json`, nos dados da Ayla: posição, fração lida e se terminou, por
//! livro; e o modo de leitura escolhido para cada série.
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Arquivos que a Biblioteca abre.
pub const KINDS: [&str; 2] = ["epub", "cbz"];
/// Limite de livros por fonte, para uma pasta enorme não travar a lista.
const MAX_BOOKS: usize = 20_000;
pub const IMPORTED: &str = "biblioteca";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Book {
    /// Caminho relativo à fonte, com `/`.
    pub path: String,
    pub title: String,
    /// Pasta do livro dentro da fonte ("" na raiz): a série.
    pub series: String,
    pub kind: String,
    pub bytes: u64,
    pub modified: i64,
    /// Livro protegido com senha (aberto nesta execução).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub protected: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    /// Onde parou: página (CBZ, a partir de 0) ou CFI (EPUB).
    pub position: String,
    /// Quanto do livro foi lido (0 a 1).
    pub fraction: f64,
    pub done: bool,
    /// Última leitura (segundos Unix).
    pub at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Reading {
    /// `fonte|caminho` → progresso.
    #[serde(default)]
    pub books: BTreeMap<String, Progress>,
    /// `fonte|série` → modo de leitura (rtl, ltr, double, vertical).
    #[serde(default)]
    pub modes: BTreeMap<String, String>,
}

pub fn kind_of(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    KINDS.into_iter().find(|k| *k == ext)
}

fn modified(meta: &fs::Metadata) -> i64 {
    meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64)
}

fn as_text(path: &Path) -> String {
    path.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// Caminho relativo seguro de um livro dentro da fonte (sem `..`, absolutos ou partes ocultas).
pub fn safe_book(relative: &str) -> Option<PathBuf> {
    let path = Path::new(relative);
    let ok = !relative.is_empty() && relative.len() <= 2048 && kind_of(path).is_some()
        && path.components().all(|c| matches!(c, Component::Normal(part) if !part.to_string_lossy().starts_with('.')));
    ok.then(|| path.to_path_buf())
}

/// Ordem natural: trechos numéricos comparados como números ("Capitulo_9" < "Capitulo_10").
pub fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |s: &str| -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = Vec::new();
        for c in s.to_lowercase().chars() {
            let digit = c.is_ascii_digit();
            match out.last_mut() {
                Some((d, text)) if *d == digit => text.push(c),
                _ => out.push((digit, c.to_string())),
            }
        }
        out
    };
    let (x, y) = (parts(a), parts(b));
    for ((dx, tx), (dy, ty)) in x.iter().zip(y.iter()) {
        let order = if *dx && *dy {
            let (nx, ny) = (tx.trim_start_matches('0'), ty.trim_start_matches('0'));
            nx.len().cmp(&ny.len()).then_with(|| nx.cmp(ny))
        } else {
            tx.cmp(ty)
        };
        if order != std::cmp::Ordering::Equal {
            return order;
        }
    }
    x.len().cmp(&y.len()).then_with(|| a.cmp(b))
}

/// Livros de uma fonte, por série e em ordem natural. Pastas e arquivos ocultos ficam de fora;
/// links simbólicos dentro da fonte não são seguidos.
pub fn scan(root: &Path) -> Vec<Book> {
    let mut out = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(root.join(&dir)) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(kind) = entry.file_type() else { continue };
            if name.starts_with('.') || kind.is_symlink() {
                continue;
            }
            let relative = dir.join(&name);
            if kind.is_dir() {
                pending.push(relative);
            } else if let (true, Some(book_kind)) = (kind.is_file(), kind_of(&relative)) {
                let Ok(meta) = entry.metadata() else { continue };
                let title = relative.file_stem().map(|s| s.to_string_lossy().replace('_', " ")).unwrap_or_default();
                out.push(Book { path: as_text(&relative), title, series: as_text(&dir), kind: book_kind.into(), bytes: meta.len(), modified: modified(&meta), protected: false });
                if out.len() >= MAX_BOOKS {
                    pending.clear();
                    break;
                }
            }
        }
    }
    out.sort_by(|a, b| natural(&a.series, &b.series).then_with(|| natural(&a.path, &b.path)));
    out
}

/// Copia livros (arquivos ou pastas inteiras, mantendo as subpastas como séries) para a pasta de
/// importados, sem sobrescrever. Devolve quantos entraram e quantos ficaram de fora.
pub fn import(target: &Path, sources: &[PathBuf]) -> Result<(usize, usize), String> {
    fs::create_dir_all(target).map_err(|_| "Não foi possível criar a Biblioteca")?;
    let mut found: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut skipped = 0;
    for source in sources {
        let Ok(meta) = fs::symlink_metadata(source) else { skipped += 1; continue };
        let Some(name) = source.file_name() else { continue };
        if meta.is_file() {
            if kind_of(source).is_some() { found.push((source.clone(), PathBuf::from(name))) } else { skipped += 1 }
            continue;
        }
        if !meta.is_dir() {
            continue;
        }
        let base = source.parent().unwrap_or(Path::new("/"));
        let mut pending = vec![PathBuf::from(name)];
        while let Some(dir) = pending.pop() {
            let Ok(entries) = fs::read_dir(base.join(&dir)) else { continue };
            for entry in entries.flatten() {
                let entry_name = entry.file_name().to_string_lossy().to_string();
                let Ok(kind) = entry.file_type() else { continue };
                if entry_name.starts_with('.') || kind.is_symlink() {
                    continue;
                }
                let relative = dir.join(&entry_name);
                if kind.is_dir() {
                    pending.push(relative);
                } else if kind.is_file() && kind_of(&relative).is_some() {
                    found.push((entry.path(), relative));
                }
            }
        }
    }
    let mut count = 0;
    for (from, relative) in found {
        let mut to = target.join(&relative);
        let stem = relative.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let ext = relative.extension().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let mut n = 2;
        while to.exists() {
            to = to.with_file_name(format!("{stem} {n}.{ext}"));
            n += 1;
        }
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|_| "Não foi possível criar a pasta do livro")?;
        }
        // Cópia num arquivo temporário: um livro pela metade nunca aparece na Biblioteca.
        let partial = to.with_file_name(format!(".{}.parcial", to.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
        match fs::copy(&from, &partial).and_then(|_| fs::rename(&partial, &to)) {
            Ok(()) => count += 1,
            Err(_) => { let _ = fs::remove_file(&partial); skipped += 1; }
        }
    }
    Ok((count, skipped))
}

/// Pastas da fonte (inclusive as vazias), para organizar os importados.
pub fn folders(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(root.join(&dir)) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let relative = dir.join(&name);
            out.push(as_text(&relative));
            pending.push(relative);
        }
    }
    out.sort_by(|a, b| natural(a, b));
    out
}

/// Caminho de pasta seguro dentro da fonte ("" é a raiz).
fn safe_dir(relative: &str) -> Option<PathBuf> {
    let path = Path::new(relative);
    let ok = relative.len() <= 2048 && path.components().all(|c| matches!(c, Component::Normal(part) if !part.to_string_lossy().starts_with('.')));
    ok.then(|| path.to_path_buf())
}

/// Cria uma pasta (nome sem barras) dentro de outra da fonte. Devolve o caminho dela.
pub fn mkdir(root: &Path, parent: &str, name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.starts_with('.') || name.contains(['/', '\\']) || name.chars().count() > 120 || name.chars().any(char::is_control) {
        return Err("Nome de pasta inválido".into());
    }
    let relative = safe_dir(parent).ok_or("Pasta inválida")?.join(name);
    fs::create_dir_all(root.join(&relative)).map_err(|_| "Não foi possível criar a pasta")?;
    Ok(as_text(&relative))
}

/// Move um livro para outra pasta da fonte, sem sobrescrever; o progresso vai junto.
pub fn move_book(root: &Path, reading: &Path, source: &str, path: &str, folder: &str) -> Result<String, String> {
    let from = safe_book(path).ok_or("Livro inválido")?;
    let name = from.file_name().ok_or("Livro inválido")?;
    let to = safe_dir(folder).ok_or("Pasta inválida")?.join(name);
    if from == to {
        return Ok(as_text(&to));
    }
    if !root.join(&from).is_file() {
        return Err("Livro não encontrado".into());
    }
    if !root.join(to.parent().unwrap_or(Path::new(""))).is_dir() {
        return Err("Pasta não encontrada".into());
    }
    if root.join(&to).exists() {
        return Err("Já existe um livro com esse nome nessa pasta".into());
    }
    fs::rename(root.join(&from), root.join(&to)).map_err(|_| "Não foi possível mover o livro")?;
    let (old, new) = (as_text(&from), as_text(&to));
    update_reading(reading, |r| {
        if let Some(progress) = r.books.remove(&key(source, &old)) {
            r.books.insert(key(source, &new), progress);
        }
    })?;
    Ok(new)
}

static READING: Mutex<()> = Mutex::new(());

pub fn load_reading(file: &Path) -> Reading {
    fs::read(file).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

fn save_reading(file: &Path, reading: &Reading) -> Result<(), String> {
    let temporary = file.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(reading).map_err(|_| "Falha ao salvar o progresso")?;
    let saved = fs::File::create(&temporary).and_then(|mut f| { f.write_all(&bytes)?; f.sync_all() }).and_then(|_| fs::rename(&temporary, file));
    saved.map_err(|_| { let _ = fs::remove_file(&temporary); "Não foi possível salvar o progresso".into() })
}

/// Altera o progresso guardado (uma alteração por vez).
pub fn update_reading(file: &Path, change: impl FnOnce(&mut Reading)) -> Result<Reading, String> {
    let _guard = READING.lock().map_err(|_| "Progresso ocupado")?;
    let mut reading = load_reading(file);
    change(&mut reading);
    save_reading(file, &reading)?;
    Ok(reading)
}

pub fn key(source: &str, path: &str) -> String {
    format!("{source}|{path}")
}

/// Livro descrito pelo caminho (para os protegidos, abertos com a senha).
pub fn book_at(path: &str, bytes: u64, modified: i64) -> Option<Book> {
    let relative = safe_book(path)?;
    let title = relative.file_stem().map(|s| s.to_string_lossy().replace('_', " ")).unwrap_or_default();
    let series = relative.parent().map(as_text).unwrap_or_default();
    Some(Book { path: as_text(&relative), title, series, kind: kind_of(&relative)?.into(), bytes, modified, protected: true })
}

/// Marca vários livros (pelas chaves do progresso) como lidos, no fim, ou não lidos, do começo,
/// sem progresso guardado. As chaves dos protegidos não têm o nome.
pub fn mark_keys(file: &Path, keys: &[String], done: bool) -> Result<(), String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    update_reading(file, |reading| {
        for book in keys {
            if done {
                let entry = reading.books.entry(book.clone()).or_default();
                entry.done = true;
                entry.fraction = 1.0;
                entry.at = now;
            } else {
                reading.books.remove(book);
            }
        }
    }).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).unwrap();
        let dir = std::env::temp_dir().join(format!("ayla-library-{}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ordem_natural_dos_capitulos() {
        let mut names = vec!["Capitulo_10.cbz", "Capitulo_9.cbz", "Capitulo_100.cbz", "capitulo_010b.cbz", "Capitulo_002.cbz"];
        names.sort_by(|a, b| natural(a, b));
        assert_eq!(names, ["Capitulo_002.cbz", "Capitulo_9.cbz", "Capitulo_10.cbz", "capitulo_010b.cbz", "Capitulo_100.cbz"]);
    }

    #[test]
    fn biblioteca_lista_series_importa_e_guarda_o_progresso() {
        let source = temp();
        let target = temp();
        fs::create_dir_all(source.join("One Piece")).unwrap();
        fs::create_dir_all(source.join(".cache")).unwrap();
        for name in ["Capitulo_10.cbz", "Capitulo_9.cbz", "OnePieceManga_Capitulo_355.cbz", "capa.jpg"] {
            fs::write(source.join("One Piece").join(name), b"PK").unwrap();
        }
        fs::write(source.join("Duna.epub"), b"PK").unwrap();
        fs::write(source.join(".cache/x.cbz"), b"PK").unwrap();
        let books = scan(&source);
        let paths: Vec<&str> = books.iter().map(|b| b.path.as_str()).collect();
        assert_eq!(paths, ["Duna.epub", "One Piece/Capitulo_9.cbz", "One Piece/Capitulo_10.cbz", "One Piece/OnePieceManga_Capitulo_355.cbz"]);
        assert_eq!((books[1].series.as_str(), books[1].title.as_str(), books[0].kind.as_str()), ("One Piece", "Capitulo 9", "epub"));
        assert!(safe_book("../x.cbz").is_none() && safe_book("a/.b/x.cbz").is_none() && safe_book("x.zip").is_none() && safe_book("a/x.cbz").is_some());

        // Importar: pasta inteira vira série; arquivo solto vai para a raiz; nada é sobrescrito.
        let (count, skipped) = import(&target, &[source.join("One Piece"), source.join("Duna.epub"), source.join("ausente.epub")]).unwrap();
        assert_eq!((count, skipped), (4, 1));
        assert!(target.join("One Piece/Capitulo_9.cbz").is_file() && target.join("Duna.epub").is_file());
        assert!(!target.join("One Piece/capa.jpg").exists());
        import(&target, &[source.join("Duna.epub")]).unwrap();
        assert!(target.join("Duna 2.epub").is_file());
        assert!(source.join("Duna.epub").is_file(), "original fica");

        let file = target.join("leitura.json");
        // Pastas criadas na Athena e livros arrastados para elas (o progresso acompanha).
        assert_eq!(mkdir(&target, "", "Ficção").unwrap(), "Ficção");
        assert_eq!(mkdir(&target, "Ficção", "Clássicos").unwrap(), "Ficção/Clássicos");
        for bad in ["", ".oculta", "a/b", "  "] { assert!(mkdir(&target, "", bad).is_err(), "{bad:?}"); }
        assert!(mkdir(&target, "../fora", "x").is_err());
        assert_eq!(folders(&target), ["Ficção", "Ficção/Clássicos", "One Piece"]);
        update_reading(&file, |r| { r.books.insert(key("ayla", "Duna.epub"), Progress { position: "cfi".into(), fraction: 0.4, done: false, at: 1 }); }).unwrap();
        assert_eq!(move_book(&target, &file, "ayla", "Duna.epub", "Ficção/Clássicos").unwrap(), "Ficção/Clássicos/Duna.epub");
        assert!(target.join("Ficção/Clássicos/Duna.epub").is_file() && !target.join("Duna.epub").exists());
        assert_eq!(load_reading(&file).books[&key("ayla", "Ficção/Clássicos/Duna.epub")].position, "cfi");
        assert!(move_book(&target, &file, "ayla", "Duna 2.epub", "Ficção/Clássicos").is_ok());
        assert!(move_book(&target, &file, "ayla", "Ficção/Clássicos/Duna.epub", "Ausente").is_err());
        fs::write(target.join("Duna.epub"), b"PK").unwrap();
        assert!(move_book(&target, &file, "ayla", "Duna.epub", "Ficção/Clássicos").is_err(), "não sobrescreve");
        assert!(move_book(&target, &file, "ayla", "Duna.epub", "../fora").is_err());
        update_reading(&file, |r| { r.modes.insert(key("mad", "One Piece"), "rtl".into()); }).unwrap();
        mark_keys(&file, &[key("mad", "One Piece/Capitulo_9.cbz"), key("mad", "One Piece/Capitulo_10.cbz")], true).unwrap();
        let reading = load_reading(&file);
        assert!(reading.books[&key("mad", "One Piece/Capitulo_10.cbz")].done && reading.books[&key("mad", "One Piece/Capitulo_10.cbz")].fraction == 1.0);
        assert_eq!(reading.modes[&key("mad", "One Piece")], "rtl", "o modo fica");
        mark_keys(&file, &[key("mad", "One Piece/Capitulo_10.cbz")], false).unwrap();
        let reading = load_reading(&file);
        assert!(!reading.books.contains_key(&key("mad", "One Piece/Capitulo_10.cbz")), "não lido volta ao começo");
        assert!(reading.books[&key("mad", "One Piece/Capitulo_9.cbz")].done);
        assert_eq!(load_reading(&file).modes[&key("mad", "One Piece")], "rtl");
        let _ = fs::remove_dir_all(source);
        let _ = fs::remove_dir_all(target);
    }
}
