//! Configuração da pessoa que viaja com os dotfiles: atalhos mudados pela Ayla, macros e rotinas
//! (só as definições; histórico e estado ficam neste computador). Mora em `~/.config/ayla`; se há
//! um repositório de dotfiles, a pasta de verdade fica nele (`<repo>/ayla/.config/ayla`, no estilo
//! do stow) e `~/.config/ayla` é um link para lá.
use std::fs;
use std::path::{Path, PathBuf};

/// Pasta da configuração. `AYLA_CONFIG_DIR` troca (testes); um perfil de teste (`AYLA_DATA_DIR`)
/// usa `<dados>/config`, para nunca mexer na configuração de verdade.
pub fn pasta() -> PathBuf {
    if let Some(dir) = std::env::var_os("AYLA_CONFIG_DIR") { return PathBuf::from(dir); }
    if let Some(dir) = std::env::var_os("AYLA_DATA_DIR") { return PathBuf::from(dir).join("config"); }
    let home = super::hand::home();
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    config.join("ayla")
}

/// Cria a pasta na primeira vez: dentro dos dotfiles (com o link) quando houver repositório.
pub fn preparar() -> Result<PathBuf, String> {
    let dir = pasta();
    if dir.exists() || fs::symlink_metadata(&dir).is_ok() { return Ok(dir); }
    let fora_de_teste = std::env::var_os("AYLA_CONFIG_DIR").is_none() && std::env::var_os("AYLA_DATA_DIR").is_none();
    if fora_de_teste {
        if let Some(repo) = super::hand_dotfiles::achar() {
            let real = repo.join("ayla/.config/ayla");
            fs::create_dir_all(&real).map_err(|_| "Não foi possível criar a pasta da Ayla nos dotfiles")?;
            if let Some(parent) = dir.parent() { fs::create_dir_all(parent).map_err(|_| "Não foi possível criar ~/.config")?; }
            let alvo = relativo(dir.parent().unwrap_or(Path::new("/")), &real);
            std::os::unix::fs::symlink(&alvo, &dir).map_err(|_| "Não foi possível criar o link ~/.config/ayla")?;
            return Ok(dir);
        }
    }
    fs::create_dir_all(&dir).map_err(|_| "Não foi possível criar a pasta de configuração da Ayla")?;
    Ok(dir)
}

/// Caminho relativo de `base` até `alvo` (os dois absolutos), como os links do stow.
pub fn relativo(base: &Path, alvo: &Path) -> PathBuf {
    let b: Vec<_> = base.components().collect();
    let a: Vec<_> = alvo.components().collect();
    let comum = b.iter().zip(&a).take_while(|(x, y)| x == y).count();
    let mut out = PathBuf::new();
    for _ in comum..b.len() { out.push(".."); }
    for c in &a[comum..] { out.push(c); }
    out
}

/// Traz um arquivo antigo dos dados da Ayla para a configuração (uma vez), deixando uma cópia
/// `.migrado` no lugar antigo.
pub fn migrar(antigo: &Path, novo: &Path) {
    if novo.exists() || !antigo.exists() { return; }
    if let Some(d) = novo.parent() { let _ = fs::create_dir_all(d); }
    if fs::copy(antigo, novo).is_ok() { let _ = fs::rename(antigo, antigo.with_extension("json.migrado")); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_relativo_como_o_stow() {
        assert_eq!(relativo(Path::new("/home/x/.config"), Path::new("/home/x/dotfiles/ayla/.config/ayla")), PathBuf::from("../dotfiles/ayla/.config/ayla"));
        assert_eq!(relativo(Path::new("/a/b"), Path::new("/a/b/c")), PathBuf::from("c"));
    }

    #[test]
    fn migra_uma_vez_com_copia() {
        let root = std::env::temp_dir().join(format!("ayla-config-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("dados")).unwrap();
        fs::write(root.join("dados/macros.json"), "[1]").unwrap();
        migrar(&root.join("dados/macros.json"), &root.join("config/macros.json"));
        assert_eq!(fs::read_to_string(root.join("config/macros.json")).unwrap(), "[1]");
        assert!(root.join("dados/macros.json.migrado").exists() && !root.join("dados/macros.json").exists());
        fs::write(root.join("dados/macros.json"), "[2]").unwrap();
        migrar(&root.join("dados/macros.json"), &root.join("config/macros.json"));
        assert_eq!(fs::read_to_string(root.join("config/macros.json")).unwrap(), "[1]", "não sobrescreve");
        fs::remove_dir_all(&root).unwrap();
    }
}
