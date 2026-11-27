//! Hand · atalhos: todos os atalhos do Hyprland (os dos dotfiles e os seus), lidos dos arquivos de
//! configuração, e as mudanças feitas pela Ayla. A Ayla só escreve no arquivo dela
//! (`custom/ayla.conf`), carregado por último: desativar ou trocar um atalho dos dotfiles vira
//! `unbind` + o atalho novo lá, sem tocar no original. Desfazer é tirar a mudança.
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Pasta de configuração do Hyprland. `AYLA_HYPR_DIR` troca a pasta (testes) e desliga o reload.
pub fn pasta_hypr() -> PathBuf {
    if let Some(dir) = std::env::var_os("AYLA_HYPR_DIR") { return PathBuf::from(dir); }
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| super::hand::home().join(".config")).join("hypr")
}

/// Com `AYLA_HYPR_DIR` (perfil de teste), nada de `hyprctl reload` na sessão de verdade.
pub fn recarregar_hyprland() -> bool { std::env::var_os("AYLA_HYPR_DIR").is_none() }

pub fn arquivo_ayla(hypr: &Path) -> PathBuf { hypr.join("custom").join("ayla.conf") }

/// Combinação de teclas: modificadores normalizados e a tecla como escrita.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Combo {
    pub mods: Vec<String>,
    pub tecla: String,
}

const ORDEM: &[&str] = &["SUPER", "CTRL", "ALT", "SHIFT", "CAPS", "MOD2", "MOD3", "MOD5"];

/// "Ctrl+Super+Alt", "SUPER_SHIFT", "$mainMod SHIFT" (já trocado) → ["SUPER", "CTRL", "ALT"].
pub fn ler_mods(text: &str) -> Vec<String> {
    let up = text.to_uppercase();
    let tokens: Vec<&str> = up.split(|c: char| !c.is_ascii_alphanumeric()).filter(|t| !t.is_empty()).collect();
    let mut out = HashSet::new();
    for t in tokens {
        let m = match t {
            "SUPER" | "WIN" | "LOGO" | "MOD4" | "META" => "SUPER",
            "CTRL" | "CONTROL" => "CTRL",
            "ALT" | "MOD1" => "ALT",
            "SHIFT" => "SHIFT",
            "CAPS" => "CAPS",
            "MOD2" => "MOD2",
            "MOD3" => "MOD3",
            "MOD5" => "MOD5",
            _ => continue,
        };
        out.insert(m);
    }
    ORDEM.iter().filter(|m| out.contains(*m)).map(|m| m.to_string()).collect()
}

impl Combo {
    pub fn novo(mods: &str, tecla: &str) -> Self { Combo { mods: ler_mods(mods), tecla: tecla.trim().to_string() } }
    pub fn igual(&self, other: &Combo) -> bool { self.mods == other.mods && self.tecla.eq_ignore_ascii_case(&other.tecla) }
    fn mods_texto(&self) -> String { self.mods.join(" ") }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Fonte {
    /// Dos dotfiles (`hyprland/…`): voltam a cada atualização deles.
    Dotfiles,
    /// Os seus (`custom/…` e o resto da configuração).
    Pessoal,
    Ayla,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Estado { Ativo, Desativado, Trocado }

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Atalho {
    pub id: String,
    pub combo: Combo,
    pub flags: String,
    pub descricao: String,
    pub acao: String,
    pub arg: String,
    pub fonte: Fonte,
    pub arquivo: String,
    pub secao: String,
    /// Interno dos dotfiles ("# [hidden]"): fica fora da lista principal.
    pub oculto: bool,
    pub submapa: String,
    pub estado: Estado,
    /// Para atalhos da Ayla: o combo do atalho original que este substitui.
    pub substitui: Option<Combo>,
    /// Resumo da cola de atalhos dos dotfiles (`#/# bind = Super, Hash,, # Focus workspace`) a
    /// que este atalho interno pertence: a lista mostra o grupo numa linha só.
    pub grupo: Option<Grupo>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Grupo {
    pub id: String,
    pub combo: Combo,
    pub descricao: String,
}

/// Linha de resumo da cola (`#/# bind = …`), sem o `#/#`; `None` se for interna.
pub fn ler_resumo(rest: &str, id: String) -> Option<Grupo> {
    let Some(Linha::Bind { combo, comentario, .. }) = ler_linha(rest.trim()) else { return None };
    if comentario.contains("[hidden]") { return None; }
    let descricao = comentario.split('#').map(str::trim).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ");
    Some(Grupo { id, combo, descricao })
}

/// O que a Ayla mudou (guardado nos dados dela; o `ayla.conf` é gerado a partir disto).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Mudancas {
    #[serde(default)]
    pub desativados: Vec<Combo>,
    #[serde(default)]
    pub novos: Vec<Novo>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Novo {
    pub id: String,
    pub combo: Combo,
    #[serde(default)]
    pub flags: String,
    pub descricao: String,
    pub acao: String,
    pub arg: String,
    #[serde(default)]
    pub substitui: Option<Combo>,
}

/// Linha `bind…` ou `unbind` lida de um arquivo.
#[derive(Clone, Debug, PartialEq)]
pub enum Linha {
    Bind { flags: String, combo: Combo, descricao: String, acao: String, arg: String, comentario: String },
    Unbind(Combo),
}

/// Separa o comentário (`#`, com `##` como # literal).
fn sem_comentario(line: &str) -> (String, String) {
    let mut out = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '#' {
            if chars.peek() == Some(&'#') { chars.next(); out.push('#'); continue; }
            return (out, chars.collect::<String>().trim().to_string());
        }
        out.push(c);
    }
    (out, String::new())
}

/// Lê uma linha de configuração (com variáveis já trocadas).
pub fn ler_linha(line: &str) -> Option<Linha> {
    let (content, comment) = sem_comentario(line);
    let (key, value) = content.split_once('=')?;
    let key = key.trim();
    if key == "unbind" {
        let mut p = value.splitn(2, ',');
        return Some(Linha::Unbind(Combo::novo(p.next()?, p.next()?.trim())));
    }
    let flags = key.strip_prefix("bind")?;
    if !flags.chars().all(|c| c.is_ascii_lowercase()) { return None; }
    let com_descricao = flags.contains('d');
    let parts = if com_descricao { 5 } else { 4 };
    let mut p = value.splitn(parts, ',').map(str::trim);
    let mods = p.next()?;
    let tecla = p.next()?;
    let descricao = if com_descricao { p.next().unwrap_or("").to_string() } else { String::new() };
    let acao = p.next().unwrap_or("").to_string();
    let arg = p.next().unwrap_or("").to_string();
    Some(Linha::Bind { flags: flags.to_string(), combo: Combo::novo(mods, tecla), descricao, acao, arg, comentario: comment })
}

fn trocar_variaveis(line: &str, vars: &[(String, String)]) -> String {
    let mut out = line.to_string();
    // As mais longas primeiro ($mainModShift antes de $mainMod).
    for (name, value) in vars { out = out.replace(&format!("${name}"), value); }
    out
}

fn expandir(path: &str, base: &Path) -> PathBuf {
    let path = path.trim();
    if let Some(rest) = path.strip_prefix("~/") { return super::hand::home().join(rest); }
    let p = PathBuf::from(path);
    if p.is_absolute() { p } else { base.join(p) }
}

/// Lê a configuração inteira (seguindo `source =`), menos o arquivo da Ayla.
pub fn ler_configuracao(hypr: &Path) -> Vec<Atalho> {
    let ayla = arquivo_ayla(hypr);
    let mut out: Vec<Atalho> = Vec::new();
    let mut vars: Vec<(String, String)> = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![hypr.join("hyprland.conf")];
    // Profundidade em ordem: cada `source` é lido no lugar em que aparece.
    fn ler(path: &Path, hypr: &Path, ayla: &Path, out: &mut Vec<Atalho>, vars: &mut Vec<(String, String)>, seen: &mut HashSet<PathBuf>, depth: usize) {
        if depth > 8 || !seen.insert(path.to_path_buf()) || path == ayla { return; }
        let Ok(text) = fs::read_to_string(path) else { return };
        let fonte = if path.strip_prefix(hypr).is_ok_and(|r| r.starts_with("hyprland")) { Fonte::Dotfiles } else { Fonte::Pessoal };
        let arquivo = path.strip_prefix(hypr).map(|r| r.to_string_lossy().into_owned()).unwrap_or_else(|_| path.to_string_lossy().into_owned());
        let (mut secao, mut submapa) = (String::new(), String::new());
        let mut grupo: Option<Grupo> = None;
        for (numero, raw) in text.lines().enumerate() {
            let trimmed = raw.trim();
            if let Some(title) = trimmed.strip_prefix("##!") { secao = title.trim().to_string(); grupo = None; continue; }
            if let Some(rest) = trimmed.strip_prefix("#/#") { grupo = ler_resumo(&trocar_variaveis(rest, vars), format!("{arquivo}#{numero}")); continue; }
            // Resumo comentado ("# #/# …"): o grupo anterior acabou.
            if trimmed.starts_with('#') && trimmed.contains("#/#") { grupo = None; continue; }
            if trimmed.starts_with('#') || trimmed.is_empty() { continue; }
            if let Some(rest) = trimmed.strip_prefix('$') {
                if let Some((name, value)) = rest.split_once('=') {
                    let value = sem_comentario(value).0.trim().to_string();
                    let value = trocar_variaveis(&value, vars);
                    vars.retain(|(n, _)| n != name.trim());
                    vars.push((name.trim().to_string(), value));
                    vars.sort_by_key(|(n, _)| std::cmp::Reverse(n.len()));
                }
                continue;
            }
            let line = trocar_variaveis(trimmed, vars);
            if let Some((key, value)) = line.split_once('=') {
                match key.trim() {
                    "source" => {
                        let target = expandir(&sem_comentario(value).0, path.parent().unwrap_or(hypr));
                        // `source` com curinga: os arquivos em ordem.
                        if target.to_string_lossy().contains('*') {
                            if let Some(dir) = target.parent() {
                                let mut files: Vec<PathBuf> = fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "conf")).collect();
                                files.sort();
                                for f in files { ler(&f, hypr, ayla, out, vars, seen, depth + 1); }
                            }
                        } else { ler(&target, hypr, ayla, out, vars, seen, depth + 1); }
                        continue;
                    }
                    "submap" => { let v = sem_comentario(value).0.trim().to_string(); submapa = if v == "reset" { String::new() } else { v }; continue; }
                    _ => {}
                }
            }
            match ler_linha(&line) {
                Some(Linha::Unbind(combo)) => {
                    for a in out.iter_mut().filter(|a| a.combo.igual(&combo) && a.submapa == submapa) { a.estado = Estado::Desativado; }
                }
                Some(Linha::Bind { flags, combo, descricao, acao, arg, comentario }) => {
                    let oculto = comentario.contains("[hidden]");
                    let descricao = if !descricao.is_empty() { descricao } else { comentario.replace("[hidden]", "").trim().to_string() };
                    let id = format!("{arquivo}:{}:{}:{}:{}", combo.mods_texto(), combo.tecla, acao, arg);
                    let grupo = if oculto { grupo.clone() } else { None };
                    out.push(Atalho { id, combo, flags, descricao, acao, arg, fonte, arquivo: arquivo.clone(), secao: secao.clone(), oculto, submapa: submapa.clone(), estado: Estado::Ativo, substitui: None, grupo });
                }
                None => {}
            }
        }
    }
    while let Some(path) = stack.pop() { ler(&path, hypr, &ayla, &mut out, &mut vars, &mut seen, 0); }
    out
}

/// A lista completa: a configuração com as mudanças da Ayla aplicadas por cima.
pub fn lista(hypr: &Path, mudancas: &Mudancas) -> Vec<Atalho> {
    let mut out = ler_configuracao(hypr);
    for a in out.iter_mut().filter(|a| a.submapa.is_empty() && a.estado == Estado::Ativo) {
        if mudancas.desativados.iter().any(|c| c.igual(&a.combo)) {
            a.estado = if mudancas.novos.iter().any(|n| n.substitui.as_ref().is_some_and(|s| s.igual(&a.combo))) { Estado::Trocado } else { Estado::Desativado };
        }
    }
    for n in &mudancas.novos {
        out.push(Atalho { id: format!("ayla:{}", n.id), combo: n.combo.clone(), flags: n.flags.clone(), descricao: n.descricao.clone(), acao: n.acao.clone(), arg: n.arg.clone(),
            fonte: Fonte::Ayla, arquivo: "custom/ayla.conf".into(), secao: "Criados pela Ayla".into(), oculto: false, submapa: String::new(), estado: Estado::Ativo, substitui: n.substitui.clone(), grupo: None });
    }
    out
}

/// Atalho ativo que já usa a combinação (fora o próprio `ignorar`).
pub fn conflito<'a>(lista: &'a [Atalho], combo: &Combo, ignorar: Option<&str>) -> Option<&'a Atalho> {
    lista.iter().find(|a| a.estado == Estado::Ativo && a.submapa.is_empty() && a.combo.igual(combo) && Some(a.id.as_str()) != ignorar)
}

fn limpo(text: &str) -> String { text.replace(['\n', '\r'], " ").trim().to_string() }

/// Confere o que vai para o arquivo: tecla e ação com caracteres esperados, sem quebra de linha.
pub fn validar(novo: &Novo) -> Result<(), String> {
    let tecla = &novo.combo.tecla;
    if tecla.is_empty() || !tecla.chars().all(|c| c.is_ascii_alphanumeric() || "_:-".contains(c)) { return Err("Tecla inválida".into()); }
    if novo.acao.is_empty() || !novo.acao.chars().all(|c| c.is_ascii_lowercase() || c == '_') { return Err("Ação do Hyprland inválida".into()); }
    if !novo.flags.chars().all(|c| "lrcgoenmtisdpuk".contains(c)) { return Err("Opções do atalho inválidas".into()); }
    if novo.arg.contains(['\n', '\r']) || novo.descricao.contains(['\n', '\r']) { return Err("Use uma linha só".into()); }
    if novo.acao == "exec" && novo.arg.trim().is_empty() { return Err("Diga o que o atalho roda".into()); }
    Ok(())
}

/// Conteúdo do `ayla.conf`: primeiro os `unbind`, depois os atalhos novos.
pub fn gerar(mudancas: &Mudancas) -> String {
    let mut out = String::from("# Gerado pela Ayla (Hand › Atalhos). Não edite: as mudanças por aqui são substituídas.\n# Desfaça pela Ayla ou apague este arquivo e a linha \"source\" dele em custom/keybinds.conf.\n\n");
    for c in &mudancas.desativados { out.push_str(&format!("unbind = {}, {}\n", c.mods_texto(), c.tecla)); }
    if !mudancas.desativados.is_empty() { out.push('\n'); }
    for n in &mudancas.novos {
        let mut flags: String = n.flags.chars().filter(|c| *c != 'd').collect();
        flags.push('d');
        // A descrição vai entre vírgulas: vírgulas nela quebrariam a linha.
        let descricao = limpo(&n.descricao).replace(',', " ·").replace('#', "");
        out.push_str(&format!("bind{flags} = {}, {}, {}, {}, {}\n", n.combo.mods_texto(), n.combo.tecla, if descricao.is_empty() { "Atalho da Ayla".into() } else { descricao }, n.acao, limpo(&n.arg).replace('#', "##")));
    }
    out
}

/// Linha que carrega o arquivo da Ayla, no fim do `custom/keybinds.conf`.
pub const FONTE_AYLA: &str = "source = ~/.config/hypr/custom/ayla.conf # Atalhos da Ayla";

/// Grava o `ayla.conf` e, na primeira vez, inclui a linha dele no `custom/keybinds.conf` (com
/// cópia antes). Recarrega o Hyprland e desfaz se ele acusar erro no arquivo da Ayla.
pub fn aplicar(hypr: &Path, mudancas: &Mudancas, backups: &Path, recarregar: bool) -> Result<(), String> {
    let file = arquivo_ayla(hypr);
    fs::create_dir_all(file.parent().unwrap_or(hypr)).map_err(|_| "Não foi possível criar a pasta dos atalhos")?;
    let previous = fs::read_to_string(&file).ok();
    let tmp = file.with_extension("conf.tmp");
    fs::write(&tmp, gerar(mudancas)).map_err(|_| "Não foi possível gravar os atalhos")?;
    fs::rename(&tmp, &file).map_err(|_| "Não foi possível gravar os atalhos")?;
    let custom = hypr.join("custom").join("keybinds.conf");
    let text = fs::read_to_string(&custom).unwrap_or_default();
    if !text.lines().any(|l| l.trim_start().starts_with("source") && l.contains("custom/ayla.conf")) {
        fs::create_dir_all(backups).map_err(|_| "Não foi possível guardar a cópia")?;
        if custom.exists() {
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            fs::copy(&custom, backups.join(format!("hypr-keybinds-{stamp}.conf"))).map_err(|_| "Não foi possível guardar a cópia do keybinds.conf")?;
        }
        let mut new = text.clone();
        if !new.is_empty() && !new.ends_with('\n') { new.push('\n'); }
        let line = if hypr.starts_with(super::hand::home().join(".config/hypr")) { FONTE_AYLA.to_string() } else { format!("source = {} # Atalhos da Ayla", file.display()) };
        new.push_str(&format!("\n{line}\n"));
        fs::write(&custom, new).map_err(|_| "Não foi possível incluir os atalhos da Ayla no keybinds.conf")?;
    }
    if !recarregar { return Ok(()); }
    let _ = Command::new("hyprctl").arg("reload").output();
    let errors = Command::new("hyprctl").arg("configerrors").output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    if errors.contains("ayla.conf") {
        match previous { Some(p) => { let _ = fs::write(&file, p); } None => { let _ = fs::write(&file, gerar(&Mudancas::default())); } }
        let _ = Command::new("hyprctl").arg("reload").output();
        return Err(format!("O Hyprland recusou o atalho: {}", errors.lines().find(|l| l.contains("ayla.conf")).unwrap_or("").trim()));
    }
    Ok(())
}

pub fn carregar(path: &Path) -> Mudancas {
    fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn guardar(path: &Path, mudancas: &Mudancas) -> Result<(), String> {
    if let Some(dir) = path.parent() { fs::create_dir_all(dir).map_err(|_| "Não foi possível guardar os atalhos")?; }
    fs::write(path, serde_json::to_vec_pretty(mudancas).unwrap_or_default()).map_err(|_| "Não foi possível guardar os atalhos".into())
}

/// Desativa um atalho da configuração (ou tira um criado pela Ayla).
pub fn desativar(mudancas: &mut Mudancas, atalho: &Atalho) {
    if atalho.fonte == Fonte::Ayla {
        mudancas.novos.retain(|n| format!("ayla:{}", n.id) != atalho.id);
        return;
    }
    if !mudancas.desativados.iter().any(|c| c.igual(&atalho.combo)) { mudancas.desativados.push(atalho.combo.clone()); }
}

/// Volta um atalho da configuração ao original (tira o `unbind` e o substituto da Ayla).
pub fn restaurar(mudancas: &mut Mudancas, combo: &Combo) {
    mudancas.desativados.retain(|c| !c.igual(combo));
    mudancas.novos.retain(|n| !n.substitui.as_ref().is_some_and(|s| s.igual(combo)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ayla-atalhos-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("hyprland")).unwrap();
        fs::create_dir_all(dir.join("custom")).unwrap();
        dir
    }

    #[test]
    fn le_linhas_de_bind() {
        assert_eq!(ler_linha("bind = Ctrl+Super, Slash, exec, xdg-open ~/a.json # Edit shell config"), Some(Linha::Bind {
            flags: String::new(), combo: Combo { mods: vec!["SUPER".into(), "CTRL".into()], tecla: "Slash".into() }, descricao: String::new(),
            acao: "exec".into(), arg: "xdg-open ~/a.json".into(), comentario: "Edit shell config".into() }));
        let Some(Linha::Bind { flags, descricao, acao, arg, .. }) = ler_linha("bindd = Super, V, Clipboard history >> clipboard, global, quickshell:overviewClipboardToggle # x") else { panic!() };
        assert_eq!((flags.as_str(), descricao.as_str(), acao.as_str(), arg.as_str()), ("d", "Clipboard history >> clipboard", "global", "quickshell:overviewClipboardToggle"));
        let Some(Linha::Bind { arg, .. }) = ler_linha("bind = SUPER, T, exec, kitty -e sh -c 'a, b' ## not comment") else { panic!() };
        assert_eq!(arg, "kitty -e sh -c 'a, b' # not comment", "vírgulas no comando e ## literal");
        assert_eq!(ler_linha("unbind = SUPER SHIFT, Q"), Some(Linha::Unbind(Combo { mods: vec!["SUPER".into(), "SHIFT".into()], tecla: "Q".into() })));
        assert_eq!(ler_linha("monitor = DP-1, 1920x1080"), None);
        assert_eq!(ler_mods("SUPER_SHIFT"), vec!["SUPER", "SHIFT"]);
        assert_eq!(ler_mods("Ctrl+Super+Alt"), vec!["SUPER", "CTRL", "ALT"]);
        assert!(Combo::novo("super", "t").igual(&Combo::novo("SUPER", "T")));
    }

    #[test]
    fn le_a_configuracao_com_source_variaveis_e_unbind() {
        let dir = temp("config");
        fs::write(dir.join("hyprland.conf"), "$mod = SUPER\nsource=hyprland/keybinds.conf\nsource=custom/keybinds.conf\n").unwrap();
        fs::write(dir.join("hyprland/keybinds.conf"), "##! Janelas\nbind = $mod, Q, killactive, # Fechar janela\nbind = $mod, T, exec, kitty # [hidden] Terminal\nsubmap = resize\nbind = , Escape, submap, reset\nsubmap = reset\n").unwrap();
        fs::write(dir.join("custom/keybinds.conf"), "unbind = SUPER, T\nbind = SUPER, B, exec, zen # Navegador\n").unwrap();
        let list = ler_configuracao(&dir);
        assert_eq!(list.len(), 4);
        let q = list.iter().find(|a| a.combo.tecla == "Q").unwrap();
        assert_eq!((q.fonte, q.secao.as_str(), q.descricao.as_str(), q.estado), (Fonte::Dotfiles, "Janelas", "Fechar janela", Estado::Ativo));
        let t = list.iter().find(|a| a.combo.tecla == "T").unwrap();
        assert!(t.oculto);
        assert_eq!(t.estado, Estado::Desativado, "unbind no arquivo pessoal");
        assert_eq!(list.iter().find(|a| a.combo.tecla == "Escape").unwrap().submapa, "resize");
        assert_eq!(list.iter().find(|a| a.combo.tecla == "B").unwrap().fonte, Fonte::Pessoal);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn atalhos_internos_entram_no_grupo_da_cola() {
        let dir = temp("grupos");
        fs::write(dir.join("hyprland.conf"), "source=hyprland/keybinds.conf\n").unwrap();
        fs::write(dir.join("hyprland/keybinds.conf"), "##! Workspace\n#/# bind = Super, Hash,, # Focus workspace # (1, 2, 3,...)\nbind = Super, code:10, exec, ws.sh workspace 1 # [hidden]\n\nbind = Super, code:11, exec, ws.sh workspace 2 # [hidden]\nbind = Super, S, togglespecialworkspace, # Scratchpad\n# #/# bind = Super, Scroll,, # x\nbind = Super, mouse_down, workspace, -1 # [hidden]\n").unwrap();
        let list = ler_configuracao(&dir);
        let g: Vec<_> = list.iter().filter_map(|a| a.grupo.as_ref().map(|g| (a.combo.tecla.as_str(), g.descricao.as_str(), g.combo.tecla.as_str()))).collect();
        assert_eq!(g, vec![("code:10", "Focus workspace (1, 2, 3,...)", "Hash"), ("code:11", "Focus workspace (1, 2, 3,...)", "Hash")]);
        assert!(list.iter().find(|a| a.combo.tecla == "S").unwrap().grupo.is_none(), "visível não entra no grupo");
        assert!(list.iter().find(|a| a.combo.tecla == "mouse_down").unwrap().grupo.is_none(), "resumo comentado encerra o grupo");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn mudancas_da_ayla_vao_para_o_arquivo_dela_por_ultimo() {
        let dir = temp("mudancas");
        fs::write(dir.join("hyprland.conf"), "source=hyprland/keybinds.conf\nsource=custom/keybinds.conf\n").unwrap();
        fs::write(dir.join("hyprland/keybinds.conf"), "bind = SUPER, Q, killactive, # Fechar\nbind = SUPER, E, exec, nautilus # Arquivos\n").unwrap();
        fs::write(dir.join("custom/keybinds.conf"), "# meus\n").unwrap();
        let mut m = Mudancas::default();
        let list = lista(&dir, &m);
        let q = list.iter().find(|a| a.combo.tecla == "Q").unwrap().clone();
        let e = list.iter().find(|a| a.combo.tecla == "E").unwrap().clone();
        desativar(&mut m, &q);
        // Trocar Super+E por Super+Shift+E.
        assert!(validar(&Novo { id: "d".into(), combo: Combo::novo("SUPER", "N"), flags: "ld".into(), descricao: "x".into(), acao: "global".into(), arg: "a:b".into(), substitui: None }).is_ok(), "trocar a tecla de um bindd/bindl dos dotfiles");
        let novo = Novo { id: "1".into(), combo: Combo::novo("SUPER SHIFT", "E"), flags: e.flags.clone(), descricao: e.descricao.clone(), acao: e.acao.clone(), arg: e.arg.clone(), substitui: Some(e.combo.clone()) };
        assert!(validar(&novo).is_ok());
        desativar(&mut m, &e);
        m.novos.push(novo);
        assert!(conflito(&lista(&dir, &m), &Combo::novo("SUPER", "Q"), None).is_none(), "desativado não conflita");
        assert!(conflito(&lista(&dir, &m), &Combo::novo("SUPER SHIFT", "E"), None).is_some());
        let list = lista(&dir, &m);
        assert_eq!(list.iter().find(|a| a.combo.tecla == "Q" && a.fonte == Fonte::Dotfiles).unwrap().estado, Estado::Desativado);
        assert_eq!(list.iter().find(|a| a.combo.tecla == "E" && a.fonte == Fonte::Dotfiles).unwrap().estado, Estado::Trocado);
        let text = gerar(&m);
        let unbind = text.find("unbind = SUPER, Q").unwrap();
        let bind = text.find("bindd = SUPER SHIFT, E, Arquivos, exec, nautilus").unwrap();
        assert!(unbind < bind && text.contains("unbind = SUPER, E"));
        aplicar(&dir, &m, &dir.join("backups"), false).unwrap();
        let custom = fs::read_to_string(dir.join("custom/keybinds.conf")).unwrap();
        assert!(custom.starts_with("# meus\n") && custom.contains("ayla.conf"), "{custom}");
        assert_eq!(fs::read_dir(dir.join("backups")).unwrap().count(), 1, "cópia antes de mexer");
        aplicar(&dir, &m, &dir.join("backups"), false).unwrap();
        assert_eq!(fs::read_to_string(dir.join("custom/keybinds.conf")).unwrap().matches("ayla.conf").count(), 1, "a linha entra uma vez só");
        restaurar(&mut m, &e.combo);
        assert!(m.novos.is_empty() && m.desativados.len() == 1);
        assert!(validar(&Novo { id: "x".into(), combo: Combo::novo("SUPER", "A\nexec"), flags: String::new(), descricao: String::new(), acao: "exec".into(), arg: "x".into(), substitui: None }).is_err());
        assert!(validar(&Novo { id: "x".into(), combo: Combo::novo("SUPER", "A"), flags: String::new(), descricao: String::new(), acao: "exec".into(), arg: "a\nbind = x".into(), substitui: None }).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}
