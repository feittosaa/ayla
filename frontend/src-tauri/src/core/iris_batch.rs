//! Iris: ações em lote (upscale, mover e lixeira). Aqui ficam as partes de caminho e de pasta, sem
//! estado da Ayla: o nome da pasta das cópias, as imagens de uma pasta e o mover sem sobrescrever.
use std::fs;
use std::path::{Path, PathBuf};

/// Máximo de imagens num upscale em lote (uma pasta enorme não vira uma fila de dias).
pub const MAX_IMAGES: usize = 2_000;

/// "Fotos" → "Fotos (upscale 4x)".
pub fn upscale_folder_name(name: &str, scale: u32) -> String {
    format!("{name} (upscale {scale}x)")
}

/// Pasta que já é saída de um upscale ("… (upscale 4x)"): não entra de novo num upscale de pasta.
pub fn is_upscale_output(name: &str) -> bool {
    name.strip_suffix("x)").and_then(|rest| rest.rsplit_once(" (upscale "))
        .is_some_and(|(stem, scale)| !stem.is_empty() && !scale.is_empty() && scale.chars().all(|c| c.is_ascii_digit()))
}

/// Pasta das cópias, ao lado da pasta de origem. Se já existir, é reaproveitada (nada dentro dela
/// é sobrescrito: ver [`free_name`]).
pub fn upscale_dir(source_dir: &Path, scale: u32) -> Result<PathBuf, String> {
    let name = source_dir.file_name().ok_or("Pasta inválida")?.to_string_lossy();
    let parent = source_dir.parent().ok_or("Pasta inválida")?;
    Ok(parent.join(upscale_folder_name(&name, scale)))
}

/// Nome livre na pasta: "foto.png", "foto (2).png", "foto (3).png"…
pub fn free_name(dir: &Path, stem: &str, extension: &str) -> String {
    let mut name = format!("{stem}.{extension}");
    let mut n = 2;
    while dir.join(&name).exists() {
        name = format!("{stem} ({n}).{extension}");
        n += 1;
    }
    name
}

/// Imagens de uma pasta e das subpastas (absoluta, relativa à pasta), sem ocultas, sem atalhos, sem
/// as saídas de upscales anteriores e sem as pastas que `skip_dir` recusar (pastas protegidas).
/// `is_image` decide pelo caminho. No máximo [`MAX_IMAGES`] + 1, para quem chama poder recusar.
pub fn images_in(dir: &Path, is_image: impl Fn(&Path) -> bool, skip_dir: impl Fn(&Path) -> bool) -> Vec<(PathBuf, PathBuf)> {
    let mut out = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let Ok(entries) = fs::read_dir(dir.join(&relative)) else { continue };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else { continue };
            if name.starts_with('.') || kind.is_symlink() {
                continue;
            }
            let path = relative.join(&name);
            if kind.is_dir() {
                if !is_upscale_output(&name) && !skip_dir(&dir.join(&path)) {
                    pending.push(path);
                }
            } else if kind.is_file() && is_image(&path) {
                out.push((dir.join(&path), path));
                if out.len() > MAX_IMAGES {
                    return out;
                }
            }
        }
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

/// Move um arquivo ou uma pasta para dentro de `to_dir`, sem sobrescrever. Entre discos diferentes,
/// copia e só apaga a origem depois da cópia inteira.
pub fn move_entry(from: &Path, to_dir: &Path) -> Result<PathBuf, String> {
    let name = from.file_name().ok_or("Caminho inválido")?;
    if to_dir.starts_with(from) {
        return Err("Uma pasta não pode ir para dentro dela mesma".into());
    }
    let to = to_dir.join(name);
    if from.parent() == Some(to_dir) {
        return Err("Já está nessa pasta".into());
    }
    if to.symlink_metadata().is_ok() {
        return Err(format!("Já existe “{}” no destino", name.to_string_lossy()));
    }
    match fs::rename(from, &to) {
        Ok(()) => Ok(to),
        Err(error) if error.kind() == std::io::ErrorKind::CrossesDevices => {
            if let Err(error) = copy_all(from, &to) {
                let _ = if to.is_dir() { fs::remove_dir_all(&to) } else { fs::remove_file(&to) };
                return Err(error);
            }
            let removed = if from.is_dir() { fs::remove_dir_all(from) } else { fs::remove_file(from) };
            removed.map_err(|_| "Copiado para o destino, mas a origem não pôde ser apagada".to_string())?;
            Ok(to)
        }
        Err(_) => Err("Não foi possível mover".into()),
    }
}

fn copy_all(from: &Path, to: &Path) -> Result<(), String> {
    let meta = from.symlink_metadata().map_err(|_| "Origem indisponível")?;
    if meta.file_type().is_symlink() {
        let target = fs::read_link(from).map_err(|_| "Atalho ilegível")?;
        return std::os::unix::fs::symlink(target, to).map_err(|_| "Não foi possível copiar um atalho".into());
    }
    if meta.is_dir() {
        fs::create_dir(to).map_err(|_| "Não foi possível criar a pasta no destino")?;
        for entry in fs::read_dir(from).map_err(|_| "Pasta ilegível")?.flatten() {
            copy_all(&entry.path(), &to.join(entry.file_name()))?;
        }
        return Ok(());
    }
    fs::copy(from, to).map(|_| ()).map_err(|_| "Não foi possível copiar para o destino".into())
}

/// O caminho (relativo, com `/`) está dentro de alguma das pastas? Pasta vazia = a raiz inteira.
pub fn covered(folders: &[&str], path: &str) -> bool {
    folders.iter().any(|folder| folder.is_empty() || path == *folder || path.starts_with(&format!("{folder}/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ayla-iris-lote-{name}-{}", super::super::store::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn nomes_da_pasta_das_copias() {
        assert_eq!(upscale_dir(Path::new("/a/Fotos"), 4).unwrap(), Path::new("/a/Fotos (upscale 4x)"));
        assert!(is_upscale_output("Fotos (upscale 4x)") && is_upscale_output("Viagem 2 (upscale 12x)"));
        assert!(!is_upscale_output("Fotos (upscale x)") && !is_upscale_output(" (upscale 4x)") && !is_upscale_output("Fotos"));
        let dir = temp("nomes");
        assert_eq!(free_name(&dir, "foto", "png"), "foto.png");
        fs::write(dir.join("foto.png"), b"").unwrap();
        fs::write(dir.join("foto (2).png"), b"").unwrap();
        assert_eq!(free_name(&dir, "foto", "png"), "foto (3).png");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn imagens_da_pasta_com_subpastas_sem_ocultas_saidas_e_protegidas() {
        let dir = temp("imagens");
        for path in ["a.jpg", "b.txt", "sub/c.png", "sub/fundo/d.webp", ".oculta/e.jpg", "sub/.f.jpg",
            "Fotos (upscale 4x)/g.jpg", "trancada/h.jpg"] {
            let file = dir.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, b"x").unwrap();
        }
        std::os::unix::fs::symlink(dir.join("a.jpg"), dir.join("atalho.jpg")).unwrap();
        let found: Vec<String> = images_in(&dir, |p| p.extension().is_some_and(|e| e != "txt"), |d| d.ends_with("trancada"))
            .into_iter().map(|(abs, rel)| { assert_eq!(abs, dir.join(&rel)); rel.to_string_lossy().into_owned() }).collect();
        assert_eq!(found, ["a.jpg", "sub/c.png", "sub/fundo/d.webp"]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn mover_sem_sobrescrever_nem_entrar_em_si() {
        let dir = temp("mover");
        fs::create_dir_all(dir.join("origem/sub")).unwrap();
        fs::create_dir_all(dir.join("destino")).unwrap();
        fs::write(dir.join("origem/sub/a.jpg"), b"1").unwrap();
        fs::write(dir.join("origem/b.jpg"), b"2").unwrap();
        fs::write(dir.join("destino/b.jpg"), b"outro").unwrap();
        assert!(move_entry(&dir.join("origem/b.jpg"), &dir.join("destino")).unwrap_err().contains("Já existe"));
        assert_eq!(fs::read(dir.join("destino/b.jpg")).unwrap(), b"outro");
        assert!(move_entry(&dir.join("origem"), &dir.join("origem/sub")).unwrap_err().contains("dentro dela"));
        assert!(move_entry(&dir.join("origem/b.jpg"), &dir.join("origem")).unwrap_err().contains("Já está"));
        let moved = move_entry(&dir.join("origem/sub"), &dir.join("destino")).unwrap();
        assert_eq!(moved, dir.join("destino/sub"));
        assert_eq!(fs::read(dir.join("destino/sub/a.jpg")).unwrap(), b"1");
        assert!(!dir.join("origem/sub").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn copia_inteira_entre_discos() {
        let dir = temp("copia");
        fs::create_dir_all(dir.join("a/b")).unwrap();
        fs::write(dir.join("a/b/c.jpg"), b"c").unwrap();
        std::os::unix::fs::symlink("b/c.jpg", dir.join("a/atalho")).unwrap();
        copy_all(&dir.join("a"), &dir.join("copia")).unwrap();
        assert_eq!(fs::read(dir.join("copia/b/c.jpg")).unwrap(), b"c");
        assert_eq!(fs::read_link(dir.join("copia/atalho")).unwrap(), Path::new("b/c.jpg"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn itens_dentro_das_pastas_escolhidas() {
        assert!(covered(&["Fotos"], "Fotos/a.jpg") && covered(&["Fotos"], "Fotos") && covered(&[""], "x/y.jpg"));
        assert!(!covered(&["Fotos"], "Fotos 2/a.jpg") && !covered(&[], "a.jpg"));
    }
}
