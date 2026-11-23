//! Arquivos do computador abertos pelo "Abrir com" do gerenciador de arquivos (Athena e Iris).
//! Nada é copiado nem conectado: a Ayla guarda, só enquanto estiver aberta, quais arquivos
//! pode ler. A Athena lê e grava apenas o texto aberto; a Iris lê as mídias da mesma pasta
//! (sem subpastas), para navegar com as setas.
use std::{
    collections::{HashMap, HashSet},
    path::{Component, Path, PathBuf},
    sync::Mutex,
};

use super::athena;

/// Prefixo das raízes de arquivos abertos de fora (`aberto:<hash da pasta>`).
pub const PREFIX: &str = "aberto:";

#[derive(Default)]
pub struct Opened {
    dirs: Mutex<HashMap<String, Dir>>,
}

#[derive(Default)]
struct Dir {
    path: PathBuf,
    texts: HashSet<String>,
    media: bool,
}

pub fn root_id(dir: &Path) -> String {
    use sha2::{Digest, Sha256};
    format!("{PREFIX}{:x}", Sha256::digest(dir.as_os_str().as_encoded_bytes()))
}

/// Nome de um arquivo direto na pasta: um único componente, sem itens ocultos.
fn plain_name(name: &str) -> bool {
    let mut parts = Path::new(name).components();
    matches!((parts.next(), parts.next()), (Some(Component::Normal(part)), None) if !part.to_string_lossy().starts_with('.'))
}

/// Caminho real do arquivo pedido (links resolvidos), que precisa existir e ser um arquivo.
pub fn real_file(path: &str) -> Result<PathBuf, String> {
    let path = Path::new(path);
    if !path.is_absolute() {
        return Err("Caminho inválido".into());
    }
    let real = path.canonicalize().map_err(|_| "Arquivo não encontrado")?;
    if !real.is_file() {
        return Err("Escolha um arquivo, não uma pasta".into());
    }
    Ok(real)
}

fn split(file: &Path) -> Result<(PathBuf, String), String> {
    let dir = file.parent().ok_or("Caminho inválido")?.to_path_buf();
    let name = file.file_name().ok_or("Caminho inválido")?.to_string_lossy().into_owned();
    if !plain_name(&name) {
        return Err("A Ayla não abre arquivos ocultos".into());
    }
    Ok((dir, name))
}

pub fn is_text(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("txt"))
}

impl Opened {
    /// Libera a leitura e a gravação de um texto (`.md` ou `.txt`). Devolve a raiz e o nome.
    pub fn allow_text(&self, file: &Path) -> Result<(String, String), String> {
        if !is_text(file) {
            return Err("A Athena abre arquivos .md e .txt".into());
        }
        let (dir, name) = split(file)?;
        let id = root_id(&dir);
        let mut dirs = self.dirs.lock().map_err(|_| "Ayla ocupada")?;
        let entry = dirs.entry(id.clone()).or_insert_with(|| Dir { path: dir, ..Default::default() });
        entry.texts.insert(name.clone());
        Ok((id, name))
    }

    /// Libera as mídias (imagens, vídeos e PDFs) da pasta de `file`. Devolve a raiz, a pasta e o nome.
    pub fn allow_media(&self, file: &Path) -> Result<(String, PathBuf, String), String> {
        athena::attachment_mime(file).ok_or("A Iris abre imagens, vídeos e PDFs")?;
        let (dir, name) = split(file)?;
        let id = root_id(&dir);
        let mut dirs = self.dirs.lock().map_err(|_| "Ayla ocupada")?;
        let entry = dirs.entry(id.clone()).or_insert_with(|| Dir { path: dir.clone(), ..Default::default() });
        entry.media = true;
        Ok((id, dir, name))
    }

    /// Pasta de um texto liberado; `None` se ele não foi aberto pelo "Abrir com".
    pub fn text_dir(&self, root: &str, name: &str) -> Option<PathBuf> {
        let dirs = self.dirs.lock().ok()?;
        let dir = dirs.get(root)?;
        (plain_name(name) && dir.texts.contains(name)).then(|| dir.path.clone())
    }

    /// Mídia liberada: arquivo direto na pasta, sem links para fora e de um tipo da Iris.
    pub fn media(&self, root: &str, name: &str) -> Option<PathBuf> {
        let dir = {
            let dirs = self.dirs.lock().ok()?;
            let dir = dirs.get(root)?;
            if !dir.media || !plain_name(name) {
                return None;
            }
            dir.path.clone()
        };
        let path = dir.join(name);
        if path.canonicalize().ok()? != path || !path.is_file() {
            return None;
        }
        athena::attachment_mime(&path)?;
        Some(path)
    }
}

/// Mídias diretas de uma pasta (sem subpastas, ocultos nem links): nome, tipo, data e tamanho.
pub fn media_in(dir: &Path) -> Vec<(String, &'static str, i64, u64)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else { continue };
        if !kind.is_file() || !plain_name(&name) {
            continue;
        }
        let Some(mime) = athena::attachment_mime(Path::new(&name)) else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        let modified = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
        out.push((name, mime, modified, meta.len()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn so_libera_o_texto_aberto_e_as_midias_da_mesma_pasta() {
        let base = std::env::temp_dir().join(format!("ayla-aberto-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("sub")).unwrap();
        for name in ["nota.md", "outra.md", "foto.png", "doc.pdf", ".oculta.png", "sub/funda.png", "planilha.csv"] {
            fs::write(base.join(name), b"x").unwrap();
        }
        let base = base.canonicalize().unwrap();
        std::os::unix::fs::symlink(base.join("foto.png"), base.join("link.png")).unwrap();
        let opened = Opened::default();

        assert!(opened.allow_text(&base.join("planilha.csv")).is_err(), "só .md e .txt");
        let (root, name) = opened.allow_text(&base.join("nota.md")).unwrap();
        assert_eq!(name, "nota.md");
        assert_eq!(opened.text_dir(&root, "nota.md"), Some(base.clone()));
        assert_eq!(opened.text_dir(&root, "outra.md"), None, "outro texto da pasta não foi aberto");
        assert_eq!(opened.text_dir(&root, "../nota.md"), None);
        assert_eq!(opened.media(&root, "foto.png"), None, "abrir um texto não libera as mídias");

        let (media_root, dir, name) = opened.allow_media(&base.join("foto.png")).unwrap();
        assert_eq!((media_root.as_str(), dir.as_path(), name.as_str()), (root.as_str(), base.as_path(), "foto.png"));
        assert_eq!(opened.media(&root, "doc.pdf"), Some(base.join("doc.pdf")));
        assert_eq!(opened.media(&root, "sub/funda.png"), None, "sem subpastas");
        assert_eq!(opened.media(&root, ".oculta.png"), None);
        assert_eq!(opened.media(&root, "link.png"), None, "links ficam de fora");
        assert_eq!(opened.media(&root, "nota.md"), None, "só mídias");
        assert_eq!(opened.media("aberto:outra", "foto.png"), None);

        let mut names: Vec<_> = media_in(&base).into_iter().map(|(n, ..)| n).collect();
        names.sort();
        assert_eq!(names, ["doc.pdf", "foto.png"]);
        assert!(real_file("relativo.md").is_err());
        assert!(real_file(base.to_str().unwrap()).is_err(), "pasta não é arquivo");
        fs::remove_dir_all(&base).unwrap();
    }
}
