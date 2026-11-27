//! Hand · programas: a grade com o ícone de cada programa, o que ele faz no sistema e o que
//! aconteceria ao desinstalar. O pacote vem do `rpm` (ou do Flatpak), a lista do que sairia junto
//! vem de uma simulação do `dnf` (sem senha, nada muda) e o texto vem do modelo local, guardado
//! por versão: só programas novos ou atualizados são avaliados de novo. Desinstalar pede a senha
//! do sistema (`pkexec`), nunca toca nos essenciais e só acontece depois da confirmação.
use super::hand::{self, fold};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Origem {
    /// Pacote do Fedora (dnf/rpm).
    Sistema,
    Flatpak,
    /// Atalho sem pacote (criado à mão, AppImage, a própria Ayla): não se desinstala por aqui.
    Manual,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct App {
    pub id: String,
    pub name: String,
    pub comment: String,
    /// Arquivo do ícone (svg/png), quando achado no tema.
    pub icone: Option<String>,
    pub origem: Origem,
    /// Pacote rpm ou id do Flatpak.
    pub pacote: String,
    pub versao: String,
    /// Instalação do Flatpak ("system"/"user").
    #[serde(skip)]
    pub instalacao: String,
    /// Quando foi usado pela última vez (aproximado).
    pub uso: Option<Uso>,
}

/// Último uso, pelo horário de acesso do executável (o sistema atualiza no máximo uma vez por
/// dia, com `relatime`) e, no Flatpak, pela última gravação em `~/.var/app/<id>`.
#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
pub struct Uso {
    /// Último uso conhecido (segundos desde 1970); `None` se não foi usado desde que chegou.
    pub ultimo: Option<i64>,
    /// Quando o programa chegou (instalação ou última atualização).
    pub desde: i64,
}

/// Executável do `Exec=` (sem `env`, variáveis e aspas), procurado no PATH.
pub fn executavel(exec: &str) -> Option<PathBuf> {
    let mut tokens = exec.split_whitespace().map(|t| t.trim_matches('"').trim_matches('\''));
    let mut first = tokens.next()?;
    while first == "env" || (first.contains('=') && !first.starts_with('/')) { first = tokens.next()?; }
    let path = Path::new(first);
    if path.is_absolute() { return path.exists().then(|| path.to_path_buf()); }
    if first.contains('/') { return None; }
    let paths = std::env::var_os("PATH").unwrap_or_else(|| "/usr/local/bin:/usr/bin:/bin".into());
    std::env::split_paths(&paths).map(|d| d.join(first)).find(|p| p.is_file())
}

/// Acesso e mudança de um arquivo (seguindo links), sem ler o conteúdo.
fn tempos(path: &Path) -> Option<(i64, i64)> {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::metadata(path).ok()?;
    Some((meta.atime(), meta.ctime()))
}

/// Uso pelo executável: acesso logo depois da instalação conta como "não usado desde que chegou".
pub fn uso_por_tempos(atime: i64, ctime: i64) -> Uso {
    Uso { ultimo: (atime > ctime + 120).then_some(atime), desde: ctime }
}

/// Última gravação dentro de uma pasta (até 3 níveis, no máximo 3000 itens).
fn ultima_gravacao(dir: &Path) -> Option<i64> {
    use std::os::unix::fs::MetadataExt;
    let mut newest: Option<i64> = None;
    let mut pending = vec![(dir.to_path_buf(), 0)];
    let mut seen = 0;
    while let Some((d, depth)) = pending.pop() {
        let Ok(entries) = fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            seen += 1;
            if seen > 3000 { return newest; }
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() { if depth < 3 { pending.push((e.path(), depth + 1)); } } else { newest = Some(newest.map_or(meta.mtime(), |n| n.max(meta.mtime()))); }
        }
    }
    newest
}

fn comando_flatpak(app_dir: &Path) -> Option<String> {
    let text = fs::read_to_string(app_dir.join("current/active/metadata")).ok()?;
    text.lines().find_map(|l| l.strip_prefix("command=")).map(|c| c.trim().to_string())
}

fn uso_flatpak(id: &str, instalacao: &str) -> Option<Uso> {
    let home = hand::home();
    let base = if instalacao == "user" { home.join(".local/share/flatpak/app") } else { PathBuf::from("/var/lib/flatpak/app") };
    let app_dir = base.join(id);
    let bin = comando_flatpak(&app_dir).map(|c| app_dir.join("current/active/files/bin").join(c));
    let pelo_bin = bin.as_deref().and_then(tempos).map(|(a, c)| uso_por_tempos(a, c));
    let dados = ultima_gravacao(&home.join(".var/app").join(id));
    let desde = pelo_bin.map(|u| u.desde).or_else(|| tempos(&app_dir.join("current")).map(|(_, c)| c))?;
    let ultimo = [pelo_bin.and_then(|u| u.ultimo), dados].into_iter().flatten().max();
    Some(Uso { ultimo, desde })
}

impl App {
    /// Chave da avaliação guardada: muda quando o programa é atualizado.
    pub fn chave(&self) -> String {
        match self.origem {
            Origem::Sistema => format!("rpm:{}@{}", self.pacote, self.versao),
            Origem::Flatpak => format!("flatpak:{}@{}", self.pacote, self.versao),
            Origem::Manual => format!("manual:{}", self.id),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Avaliacao {
    /// O que o programa faz no sistema, em uma ou duas frases.
    pub resumo: String,
    /// O que acontece ao desinstalar.
    pub desinstalar: String,
    /// Pacotes que o dnf removeria junto (sem o próprio).
    #[serde(default)]
    pub junto: Vec<String>,
    #[serde(default)]
    pub libera: String,
    pub essencial: bool,
    /// Texto escrito pelo modelo local (senão, só a descrição do pacote).
    #[serde(default)]
    pub modelo: bool,
}

// ---------------------------------------------------------------------------------------------
// Ícones

/// Temas de ícones na ordem de preferência: o do KDE/Qt (usado pelo Hyprland com Qt), o do GTK,
/// os herdados por eles e o hicolor no fim.
fn temas(bases: &[PathBuf]) -> Vec<String> {
    let mut wanted = Vec::new();
    let home = hand::home();
    if let Ok(text) = fs::read_to_string(home.join(".config/kdeglobals")) {
        let mut icons = false;
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') { icons = line == "[Icons]"; continue; }
            if icons { if let Some(v) = line.strip_prefix("Theme=") { wanted.push(v.trim().to_string()); } }
        }
    }
    if let Ok(out) = Command::new("gsettings").args(["get", "org.gnome.desktop.interface", "icon-theme"]).output() {
        let v = String::from_utf8_lossy(&out.stdout).trim().trim_matches('\'').to_string();
        if !v.is_empty() { wanted.push(v); }
    }
    let mut out: Vec<String> = Vec::new();
    while let Some(theme) = (!wanted.is_empty()).then(|| wanted.remove(0)) {
        if out.contains(&theme) || out.len() > 12 { continue; }
        for base in bases {
            if let Ok(text) = fs::read_to_string(base.join(&theme).join("index.theme")) {
                if let Some(line) = text.lines().find(|l| l.starts_with("Inherits=")) {
                    wanted.extend(line["Inherits=".len()..].split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
                }
                break;
            }
        }
        out.push(theme);
    }
    for t in ["hicolor", "Adwaita", "breeze"] { if !out.iter().any(|o| o == t) { out.push(t.to_string()); } }
    out
}

fn icon_bases() -> Vec<PathBuf> {
    let home = hand::home();
    let data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
    let mut bases = vec![data_home.join("icons"), home.join(".icons"), data_home.join("flatpak/exports/share/icons")];
    let system = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    bases.extend(system.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join("icons")));
    bases.push(PathBuf::from("/var/lib/flatpak/exports/share/icons"));
    let mut seen = HashSet::new();
    bases.retain(|b| seen.insert(b.clone()));
    bases
}

/// Nota do tamanho pela pasta: vetorial é a melhor; senão, o maior até 256.
fn nota_tamanho(path: &Path) -> u32 {
    let mut best = 0;
    for c in path.components() {
        let part = c.as_os_str().to_string_lossy();
        if part == "scalable" { return 300; }
        let n: String = part.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(size) = n.parse::<u32>() { best = best.max(if size > 256 { 200 } else { size }); }
    }
    if best == 0 && path.extension().is_some_and(|e| e == "svg") { 250 } else { best }
}

/// Índice nome → arquivo: o primeiro tema que tem o ícone ganha; dentro dele, o melhor tamanho.
pub struct Icones(HashMap<String, PathBuf>);

impl Icones {
    pub fn carregar() -> Self {
        let bases = icon_bases();
        let mut found: HashMap<String, (usize, u32, PathBuf)> = HashMap::new();
        for (rank, theme) in temas(&bases).iter().enumerate() {
            for base in &bases {
                let dir = base.join(theme);
                if !dir.is_dir() { continue; }
                let mut pending = vec![(dir, 0)];
                while let Some((dir, depth)) = pending.pop() {
                    let Ok(entries) = fs::read_dir(&dir) else { continue };
                    for entry in entries.flatten() {
                        let path = entry.path();
                        let name = entry.file_name().to_string_lossy().into_owned();
                        if path.is_dir() {
                            // Só as categorias que têm ícones de programas.
                            if depth < 3 && !matches!(name.as_str(), "actions" | "emblems" | "emotes" | "status" | "mimetypes" | "animations" | "stock" | "cursors") { pending.push((path, depth + 1)); }
                            continue;
                        }
                        let Some((stem, ext)) = name.rsplit_once('.') else { continue };
                        if !matches!(ext, "svg" | "png") || stem.ends_with("-symbolic") { continue; }
                        let score = nota_tamanho(&path);
                        let better = match found.get(stem) { None => true, Some((r, s, _)) => rank < *r || (rank == *r && score > *s) };
                        if better { found.insert(stem.to_string(), (rank, score, path)); }
                    }
                }
            }
        }
        let mut map: HashMap<String, PathBuf> = found.into_iter().map(|(k, (_, _, p))| (k, p)).collect();
        if let Ok(entries) = fs::read_dir("/usr/share/pixmaps") {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if let Some((stem, "svg" | "png")) = name.rsplit_once('.') { map.entry(stem.to_string()).or_insert(entry.path()); }
            }
        }
        Self(map)
    }

    pub fn achar(&self, icon: &str) -> Option<PathBuf> {
        if icon.is_empty() { return None; }
        let path = Path::new(icon);
        if path.is_absolute() { return path.is_file().then(|| path.to_path_buf()); }
        let stem = icon.strip_suffix(".png").or_else(|| icon.strip_suffix(".svg")).unwrap_or(icon);
        self.0.get(stem).cloned()
    }
}

/// Arquivo que pode ser servido como ícone: svg/png numa pasta de ícones, sem `..`.
pub fn icone_valido(path: &Path) -> bool {
    let text = path.to_string_lossy();
    path.is_absolute() && !path.components().any(|c| matches!(c, std::path::Component::ParentDir))
        && path.extension().is_some_and(|e| e == "svg" || e == "png")
        && (text.contains("/icons/") || text.starts_with("/usr/share/pixmaps/"))
}

// ---------------------------------------------------------------------------------------------
// Pacotes

/// `Exec=` da seção principal de um `.desktop`.
fn exec_de(text: &str) -> Option<String> {
    let mut main = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') { main = line == "[Desktop Entry]"; continue; }
        if main { if let Some(v) = line.strip_prefix("Exec=") { return Some(v.trim().to_string()); } }
    }
    None
}

/// Programas do menu com o arquivo `.desktop` e o `Exec=` de cada um.
fn desktops() -> Vec<(hand::Programa, PathBuf, String)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for dir in hand::application_dirs() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "desktop") { continue; }
            let id = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            if !seen.insert(id.clone()) { continue; }
            let Ok(text) = fs::read_to_string(&path) else { continue };
            if let Some(program) = hand::parse_desktop(&id, &text) { out.push((program, fs::canonicalize(&path).unwrap_or(path), exec_de(&text).unwrap_or_default())); }
        }
    }
    out
}

/// Linhas do `rpm -qf` com os arquivos de cada pacote: arquivo → (pacote, versão).
pub fn ler_donos(texto: &str, queridos: &HashSet<String>) -> HashMap<String, (String, String)> {
    texto.lines().filter_map(|line| {
        let mut parts = line.splitn(3, '\t');
        let (name, version, file) = (parts.next()?, parts.next()?, parts.next()?);
        queridos.contains(file).then(|| (file.to_string(), (name.to_string(), version.to_string())))
    }).collect()
}

fn donos(arquivos: &[String]) -> HashMap<String, (String, String)> {
    if arquivos.is_empty() { return HashMap::new(); }
    let wanted: HashSet<String> = arquivos.iter().cloned().collect();
    let mut out = HashMap::new();
    for chunk in arquivos.chunks(200) {
        let Ok(result) = Command::new("rpm").args(["-qf", "--qf", "[%{=NAME}\t%{=VERSION}-%{=RELEASE}\t%{FILENAMES}\n]"]).args(chunk).env("LC_ALL", "C").output() else { continue };
        out.extend(ler_donos(&String::from_utf8_lossy(&result.stdout), &wanted));
    }
    out
}

/// Apps Flatpak instalados: id → (versão, instalação).
fn flatpaks() -> HashMap<String, (String, String)> {
    let Ok(out) = Command::new("flatpak").args(["list", "--app", "--columns=application,version,installation"]).output() else { return HashMap::new() };
    String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| {
        let mut p = l.split('\t');
        let id = p.next()?.trim().to_string();
        let version = p.next().unwrap_or("").trim().to_string();
        let inst = p.next().unwrap_or("system").trim().to_string();
        (!id.is_empty()).then_some((id, (version, inst)))
    }).collect()
}

/// Índice de ícones reaproveitado por um minuto (a grade recarrega a cada lote avaliado).
static ICONES: std::sync::Mutex<Option<(std::time::Instant, std::sync::Arc<Icones>)>> = std::sync::Mutex::new(None);

fn icones() -> std::sync::Arc<Icones> {
    let mut slot = match ICONES.lock() { Ok(s) => s, Err(_) => return std::sync::Arc::new(Icones::carregar()) };
    if let Some((when, index)) = slot.as_ref() { if when.elapsed() < std::time::Duration::from_secs(60) { return index.clone(); } }
    let index = std::sync::Arc::new(Icones::carregar());
    *slot = Some((std::time::Instant::now(), index.clone()));
    index
}

/// Todos os programas do menu com ícone, origem e pacote.
pub fn listar() -> Vec<App> {
    let icones = icones();
    let items = desktops();
    let system: Vec<String> = items.iter().map(|(_, p, _)| p.to_string_lossy().into_owned()).filter(|p| !p.contains("/flatpak/")).collect();
    let owners = donos(&system);
    let flats = flatpaks();
    let mut apps: Vec<App> = items.into_iter().map(|(p, path, exec)| {
        let file = path.to_string_lossy().into_owned();
        let flatpak = file.contains("/flatpak/").then(|| flats.keys().filter(|id| p.id == **id || p.id.starts_with(&format!("{id}."))).max_by_key(|id| id.len()).cloned()).flatten();
        let (origem, pacote, versao, instalacao) = if let Some(id) = flatpak {
            let (v, inst) = flats[&id].clone();
            (Origem::Flatpak, id, v, inst)
        } else if let Some((name, version)) = owners.get(&file) {
            (Origem::Sistema, name.clone(), version.clone(), String::new())
        } else {
            (Origem::Manual, String::new(), String::new(), String::new())
        };
        let uso = if origem == Origem::Flatpak { uso_flatpak(&pacote, &instalacao) }
            else { executavel(&exec).as_deref().and_then(tempos).map(|(a, c)| uso_por_tempos(a, c)) };
        App { icone: icones.achar(&p.icon).map(|i| i.to_string_lossy().into_owned()), id: p.id, name: p.name, comment: p.comment, origem, pacote, versao, instalacao, uso }
    }).collect();
    apps.sort_by_key(|a| fold(&a.name));
    apps
}

// ---------------------------------------------------------------------------------------------
// O que sairia junto

/// Pacotes que nunca são desinstalados pela Ayla (além dos protegidos do dnf): o que mantém o
/// sistema, a rede, o som, o vídeo e a sessão de pé. Nomes exatos e famílias inteiras.
const ESSENCIAIS: &[&str] = &[
    "glibc", "bash", "coreutils", "systemd", "systemd-udev", "systemd-libs", "systemd-pam", "dnf", "dnf5", "libdnf5", "rpm", "rpm-libs", "sudo",
    "polkit", "polkit-libs", "dbus", "dbus-broker", "dbus-libs", "NetworkManager", "NetworkManager-libnm", "NetworkManager-wifi", "pipewire",
    "pipewire-libs", "pipewire-pulseaudio", "wireplumber", "wireplumber-libs", "hyprland", "hyprutils", "hyprlang", "aquamarine", "quickshell",
    "xdg-desktop-portal", "setup", "filesystem", "fedora-release", "fedora-release-common", "linux-firmware", "util-linux", "shadow-utils",
    "pam", "flatpak", "flatpak-libs", "gnome-shell", "sddm", "gdm", "selinux-policy", "selinux-policy-targeted", "mesa-dri-drivers",
    "mesa-libGL", "mesa-libEGL", "mesa-libgbm", "mesa-vulkan-drivers", "xorg-x11-server-Xwayland", "glib2", "gtk3", "gtk4", "qt6-qtbase",
    "openssl-libs", "ca-certificates", "fontconfig", "kbd", "passwd", "login",
];
const FAMILIAS: &[&str] = &["kernel", "grub2-", "shim-", "glibc-", "hyprland-", "quickshell-", "xdg-desktop-portal-", "fedora-release-"];

/// A partir de quantos pacotes junto a remoção é "grande" (o aviso fica forte).
pub const MUITOS: usize = 20;

pub fn protegidos() -> Vec<String> {
    let Ok(entries) = fs::read_dir("/etc/dnf/protected.d") else { return Vec::new() };
    entries.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "conf"))
        .flat_map(|e| fs::read_to_string(e.path()).unwrap_or_default().lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(str::to_string).collect::<Vec<_>>())
        .collect()
}

pub fn essencial(pacote: &str, protegidos: &[String]) -> bool {
    protegidos.iter().any(|p| p == pacote) || ESSENCIAIS.contains(&pacote)
        || FAMILIAS.iter().any(|f| if f.ends_with('-') { pacote.starts_with(f) } else { pacote == *f || pacote.starts_with(&format!("{f}-core")) || pacote.starts_with(&format!("{f}-modules")) })
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Simulacao {
    /// Todos os pacotes que sairiam (o próprio incluído).
    pub sairiam: Vec<String>,
    pub libera: String,
    pub instalado: bool,
    /// O dnf se recusa (pacote protegido ou que o sistema exige).
    pub recusado: bool,
}

/// Lê a saída do `dnf remove --assumeno` (dnf5 e dnf4, em inglês).
pub fn ler_simulacao(texto: &str) -> Simulacao {
    let mut sim = Simulacao { instalado: !texto.contains("No packages to remove") && !texto.contains("No match for argument"), ..Default::default() };
    let mut inside = false;
    for line in texto.lines() {
        let trimmed = line.trim();
        if !line.starts_with(' ') {
            inside = trimmed.starts_with("Removing") && trimmed.ends_with(':');
            if let Some(rest) = trimmed.strip_prefix("After this operation, ") { sim.libera = rest.split(" will be freed").next().unwrap_or("").split(" (").next().unwrap_or("").trim().to_string(); }
            if let Some(rest) = trimmed.strip_prefix("Freed space: ") { sim.libera = rest.trim().to_string(); }
            continue;
        }
        if inside {
            if let Some(name) = trimmed.split_whitespace().next() {
                if name.ends_with(':') { continue; }
                if !sim.sairiam.iter().any(|s| s == name) { sim.sairiam.push(name.to_string()); }
            }
        }
    }
    sim.recusado = sim.instalado && sim.sairiam.is_empty();
    sim
}

pub fn simular(pacote: &str) -> Result<Simulacao, String> {
    if pacote.is_empty() || pacote.starts_with('-') || !pacote.chars().all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c)) { return Err("Pacote inválido".into()); }
    let out = Command::new("dnf").args(["-C", "remove", "--assumeno", pacote]).env("LC_ALL", "C").output()
        .map_err(|_| "O dnf não está disponível")?;
    let text = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    Ok(ler_simulacao(&text))
}

/// Avaliação sem modelo: a descrição do pacote e a simulação, em texto.
pub fn avaliacao_basica(app: &App, resumo_pacote: &str, sim: Option<&Simulacao>, protegidos: &[String]) -> Avaliacao {
    let resumo = if !app.comment.is_empty() { app.comment.clone() } else { resumo_pacote.to_string() };
    match app.origem {
        Origem::Manual => Avaliacao { resumo, desinstalar: "Não veio de um pacote (atalho criado à mão, AppImage ou a própria Ayla): não dá para desinstalar por aqui.".into(), ..Default::default() },
        Origem::Flatpak => Avaliacao { resumo, desinstalar: format!("Sai só o app (Flatpak, {}). Os dados dele ficam em ~/.var/app/{}.", if app.instalacao == "user" { "só deste usuário" } else { "do sistema" }, app.pacote), ..Default::default() },
        Origem::Sistema => {
            let junto: Vec<String> = sim.map(|s| s.sairiam.iter().filter(|p| **p != app.pacote).cloned().collect()).unwrap_or_default();
            let essencial = essencial(&app.pacote, protegidos) || sim.is_some_and(|s| s.recusado || s.sairiam.iter().any(|p| essencial(p, protegidos)));
            let desinstalar = if essencial { "Faz parte do que mantém o sistema funcionando (ou levaria junto algo essencial). A Ayla não desinstala.".to_string() }
                else if junto.is_empty() { format!("Sai só ele{}. Nada mais depende dele.", sim.filter(|s| !s.libera.is_empty()).map(|s| format!(" (libera {})", s.libera)).unwrap_or_default()) }
                else if junto.len() >= MUITOS { format!("Leva muita coisa junto: {} pacotes, entre eles {}.", junto.len(), lista_curta(&junto, 5)) }
                else { format!("Sai junto: {}{}.", lista_curta(&junto, 6), sim.filter(|s| !s.libera.is_empty()).map(|s| format!(" (libera {})", s.libera)).unwrap_or_default()) };
            Avaliacao { resumo, desinstalar, junto, libera: sim.map(|s| s.libera.clone()).unwrap_or_default(), essencial, modelo: false }
        }
    }
}

fn lista_curta(items: &[String], max: usize) -> String {
    if items.len() <= max { items.join(", ") } else { format!("{} e mais {}", items[..max].join(", "), items.len() - max) }
}

/// Resumo e descrição de pacotes rpm (para o modelo avaliar).
pub fn descricoes(pacotes: &[String]) -> HashMap<String, (String, String)> {
    if pacotes.is_empty() { return HashMap::new(); }
    let Ok(out) = Command::new("rpm").arg("-q").arg("--qf").arg("%{NAME}\x1f%{SUMMARY}\x1f%{DESCRIPTION}\x1e").args(pacotes).env("LC_ALL", "C").output() else { return HashMap::new() };
    String::from_utf8_lossy(&out.stdout).split('\x1e').filter_map(|r| {
        let mut p = r.split('\x1f');
        let name = p.next()?.trim().to_string();
        Some((name, (p.next()?.trim().to_string(), p.next().unwrap_or("").trim().chars().take(400).collect())))
    }).collect()
}

/// Pedido ao modelo local para um lote de programas.
pub fn avaliacao_pedido(itens: &[(String, &App, String, String, &Avaliacao)]) -> Vec<Value> {
    let lista: Vec<Value> = itens.iter().map(|(chave, app, resumo, descricao, base)| json!({
        "chave": chave, "nome": app.name, "pacote": app.pacote, "origem": app.origem, "comentario": app.comment, "resumo_do_pacote": resumo,
        "descricao": descricao, "pacotes_que_sairiam_junto": base.junto.len(), "alguns_deles": base.junto.iter().take(25).collect::<Vec<_>>(), "essencial": base.essencial,
    })).collect();
    vec![
        json!({"role":"system","content":"Você explica programas instalados no Fedora Linux para uma pessoa leiga, em português do Brasil. Para cada programa, escreva: \"resumo\": o que ele faz neste sistema, em uma frase curta (até 20 palavras), concreta (\"Gerenciador de arquivos do GNOME\", \"Toca música do Spotify\"). \"desinstalar\": o que a pessoa perde ao desinstalar, em uma frase curta e concreta (\"Você perde o gerenciador de arquivos; as pastas continuam lá.\"). Use \"alguns_deles\" (pacotes que o sistema removeria junto) só para dizer o que mais deixaria de funcionar (\"o KDE inteiro sairia junto\", \"as máquinas virtuais param\"); não liste nomes de pacotes nem diga quantos são, a Ayla já mostra a lista. Se \"essencial\" for true, diga só que é parte do sistema. Não invente recursos; se não souber, use a descrição. Responda só o JSON."}),
        json!({"role":"user","content": serde_json::to_string(&lista).unwrap_or_default()}),
    ]
}

pub fn avaliacao_schema() -> Value {
    json!({"type":"object","properties":{"programas":{"type":"array","items":{"type":"object","properties":{
        "chave":{"type":"string"},"resumo":{"type":"string"},"desinstalar":{"type":"string"}},"required":["chave","resumo","desinstalar"]}}},"required":["programas"]})
}

/// Lê a resposta do modelo: chave → (resumo, desinstalar).
pub fn ler_avaliacao(raw: &str) -> HashMap<String, (String, String)> {
    let start = raw.find('{').unwrap_or(0);
    let value: Value = serde_json::from_str(&raw[start..]).or_else(|_| serde_json::from_str(raw)).unwrap_or(Value::Null);
    value["programas"].as_array().into_iter().flatten().filter_map(|p| {
        let (k, r, d) = (p["chave"].as_str()?, p["resumo"].as_str()?.trim(), p["desinstalar"].as_str()?.trim());
        (!r.is_empty()).then(|| (k.to_string(), (r.chars().take(240).collect(), d.chars().take(320).collect())))
    }).collect()
}

pub fn cache_load(path: &Path) -> HashMap<String, Avaliacao> {
    fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn cache_save(path: &Path, cache: &HashMap<String, Avaliacao>) -> Result<(), String> {
    if let Some(dir) = path.parent() { fs::create_dir_all(dir).map_err(|_| "Não foi possível guardar as avaliações")?; }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec(cache).unwrap_or_default()).map_err(|_| "Não foi possível guardar as avaliações")?;
    fs::rename(&tmp, path).map_err(|_| "Não foi possível guardar as avaliações".into())
}

/// Comando que desinstala (mostrado e rodado só depois da confirmação).
pub fn comando_desinstalar(app: &App) -> Option<(String, Vec<String>)> {
    match app.origem {
        Origem::Sistema => Some(("pkexec".into(), vec!["dnf".into(), "remove".into(), "-y".into(), app.pacote.clone()])),
        Origem::Flatpak => Some(("flatpak".into(), vec!["uninstall".into(), "-y".into(), "--noninteractive".into(), if app.instalacao == "user" { "--user".into() } else { "--system".into() }, app.pacote.clone()])),
        Origem::Manual => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulacao_do_dnf_lista_o_que_sairia() {
        let text = "Package  Arch Version Repository Size\nRemoving:\n nautilus x86_64 49.5-1.fc43 updates 13.8 MiB\nRemoving dependent packages:\n gnome-classic-session noarch 49.0-2 abc 17.0 KiB\n papers-nautilus x86_64 49.3-1 updates 19.5 KiB\nRemoving unused dependencies:\n nautilus-extensions x86_64 49.5-1 updates 82.7 KiB\n\nTransaction Summary:\n Removing:           4 packages\n\nAfter this operation, 15 MiB will be freed (install 0 B, remove 15 MiB).\nOperation aborted by the user.\n";
        let sim = ler_simulacao(text);
        assert_eq!(sim.sairiam, vec!["nautilus", "gnome-classic-session", "papers-nautilus", "nautilus-extensions"]);
        assert_eq!(sim.libera, "15 MiB");
        assert!(sim.instalado);
        assert!(!ler_simulacao("No packages to remove for argument: htop\n\nNothing to do.\n").instalado);
        assert!(ler_simulacao("Problem: The operation would result in removing the following protected packages: sudo\n").recusado);
    }

    #[test]
    fn essenciais_nunca_saem() {
        let prot = vec!["sudo".to_string(), "NetworkManager".to_string()];
        for p in ["kernel-core", "kernel", "kernel-modules-extra", "systemd-udev", "mesa-dri-drivers", "hyprland", "quickshell-git", "sudo", "glibc", "glibc-langpack-pt", "grub2-efi-x64"] { assert!(essencial(p, &prot), "{p}"); }
        for p in ["systemd-container", "gnome-shell-extension-apps-menu", "polkit-kde", "kernel-tools", "setup-extra", "flatpak-kcm"] { assert!(!essencial(p, &prot), "{p} não é essencial"); }
        assert!(!essencial("nautilus", &prot));
        assert!(!essencial("vlc", &prot));
        let app = App { id: "x".into(), name: "X".into(), comment: String::new(), icone: None, origem: Origem::Sistema, pacote: "nautilus".into(), versao: "1".into(), instalacao: String::new(), uso: None };
        let sim = Simulacao { sairiam: vec!["nautilus".into(), "systemd".into()], libera: "1 MiB".into(), instalado: true, recusado: false };
        assert!(avaliacao_basica(&app, "", Some(&sim), &prot).essencial, "levaria junto algo essencial");
        let sim = Simulacao { sairiam: vec!["nautilus".into(), "papers-nautilus".into()], libera: "15 MiB".into(), instalado: true, recusado: false };
        let a = avaliacao_basica(&app, "File manager", Some(&sim), &prot);
        assert!(!a.essencial);
        assert_eq!(a.junto, vec!["papers-nautilus"]);
        assert!(a.desinstalar.contains("papers-nautilus") && a.desinstalar.contains("15 MiB"));
        assert_eq!(a.resumo, "File manager");
    }

    #[test]
    fn donos_pelo_rpm_e_comandos() {
        let wanted: HashSet<String> = ["/usr/share/applications/a.desktop".to_string()].into();
        let text = "nautilus\t49.5\t/usr/bin/nautilus\nnautilus\t49.5\t/usr/share/applications/a.desktop\n";
        assert_eq!(ler_donos(text, &wanted).get("/usr/share/applications/a.desktop"), Some(&("nautilus".into(), "49.5".into())));
        let mut app = App { id: "a".into(), name: "A".into(), comment: String::new(), icone: None, origem: Origem::Sistema, pacote: "nautilus".into(), versao: "1".into(), instalacao: String::new(), uso: None };
        assert_eq!(comando_desinstalar(&app).unwrap().1, vec!["dnf", "remove", "-y", "nautilus"]);
        app.origem = Origem::Flatpak; app.pacote = "com.spotify.Client".into(); app.instalacao = "system".into();
        assert_eq!(comando_desinstalar(&app).unwrap(), ("flatpak".into(), vec!["uninstall".into(), "-y".into(), "--noninteractive".into(), "--system".into(), "com.spotify.Client".into()]));
        app.origem = Origem::Manual;
        assert!(comando_desinstalar(&app).is_none());
        assert_ne!(App { versao: "2".into(), ..app.clone() }.chave(), App { origem: Origem::Sistema, ..app }.chave());
    }

    #[test]
    fn ultimo_uso_pelo_acesso_do_executavel() {
        assert_eq!(uso_por_tempos(1_000, 1_000), Uso { ultimo: None, desde: 1_000 }, "lido só na instalação");
        assert_eq!(uso_por_tempos(1_060, 1_000).ultimo, None, "segundos depois ainda é a instalação");
        assert_eq!(uso_por_tempos(90_000, 1_000).ultimo, Some(90_000));
        assert_eq!(exec_de("[Desktop Entry]\nExec=env GDK_BACKEND=x11 sh %U\n[Desktop Action x]\nExec=outro\n").as_deref(), Some("env GDK_BACKEND=x11 sh %U"));
        assert_eq!(executavel("env GDK_BACKEND=x11 sh %U").map(|p| p.file_name().unwrap().to_owned()), Some("sh".into()));
        assert_eq!(executavel("\"/bin/sh\" -c x"), Some(PathBuf::from("/bin/sh")));
        assert!(executavel("naoexiste-ayla-xyz").is_none());
    }

    #[test]
    fn icones_validos_e_respostas_do_modelo() {
        assert!(icone_valido(Path::new("/usr/share/icons/hicolor/48x48/apps/a.png")));
        assert!(!icone_valido(Path::new("/home/x/.ssh/id_rsa")));
        assert!(!icone_valido(Path::new("/usr/share/icons/../../etc/a.png")));
        assert_eq!(nota_tamanho(Path::new("/i/hicolor/scalable/apps/a.svg")), 300);
        assert!(nota_tamanho(Path::new("/i/hicolor/128x128/apps/a.png")) > nota_tamanho(Path::new("/i/hicolor/48x48/apps/a.png")));
        let r = ler_avaliacao("{\"programas\":[{\"chave\":\"rpm:a@1\",\"resumo\":\"Gerenciador de arquivos\",\"desinstalar\":\"Você perde o navegador de pastas.\"},{\"chave\":\"x\",\"resumo\":\"\",\"desinstalar\":\"\"}]}");
        assert_eq!(r.len(), 1);
        assert_eq!(r["rpm:a@1"].0, "Gerenciador de arquivos");
    }
}
