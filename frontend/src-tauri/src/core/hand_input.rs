//! Hand · controle: o teclado, o mouse e as janelas, como uma pessoa usaria. Teclado pelo `wtype`
//! (teclado virtual do Wayland), cursor pelo `hyprctl dispatch movecursor`, cliques e rolagem pelo
//! `ydotool` (com um `ydotoold` só da Ayla, socket privado, encerrado quando ela fecha) e janelas
//! pelo `hyprctl`. Cada ação é pedida pela pessoa (página do Hand) ou aprovada por ela.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Janela {
    pub address: String,
    pub title: String,
    pub class: String,
    pub workspace: String,
    pub at: [i64; 2],
    pub size: [i64; 2],
    pub focused: bool,
}

fn hypr(args: &[&str]) -> Result<String, String> {
    let out = Command::new("hyprctl").args(args).output().map_err(|_| "O controle de janelas precisa do Hyprland (hyprctl)")?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || text.starts_with("error") || text.contains("Invalid") { return Err(format!("O Hyprland recusou: {text}")); }
    Ok(text)
}

/// Janelas abertas, a em foco primeiro (sem as da própria Ayla).
pub fn janelas() -> Result<Vec<Janela>, String> {
    let clients: serde_json::Value = serde_json::from_str(&hypr(&["clients", "-j"])?).map_err(|_| "Resposta inválida do Hyprland")?;
    let focused = hypr(&["activewindow", "-j"]).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).and_then(|w| w["address"].as_str().map(str::to_string));
    let me = u64::from(std::process::id());
    let mut list: Vec<Janela> = clients.as_array().into_iter().flatten()
        .filter(|c| c["mapped"].as_bool() != Some(false) && c["pid"].as_u64() != Some(me))
        .filter_map(|c| Some(Janela {
            address: c["address"].as_str()?.to_string(),
            title: c["title"].as_str().unwrap_or("").to_string(),
            class: c["class"].as_str().unwrap_or("").to_string(),
            workspace: c["workspace"]["name"].as_str().unwrap_or("").to_string(),
            at: [c["at"][0].as_i64()?, c["at"][1].as_i64()?],
            size: [c["size"][0].as_i64()?, c["size"][1].as_i64()?],
            focused: focused.as_deref() == c["address"].as_str(),
        }))
        .collect();
    list.sort_by_key(|j| !j.focused);
    Ok(list)
}

fn address_ok(address: &str) -> bool {
    address.starts_with("0x") && address.len() <= 20 && address[2..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Traz a janela para a frente (troca de workspace, se preciso).
pub fn focar(address: &str) -> Result<(), String> {
    if !address_ok(address) { return Err("Janela inválida".into()); }
    hypr(&["dispatch", "focuswindow", &format!("address:{address}")]).map(|_| ())
}

/// Fecha a janela (como o X dela; o programa pode perguntar antes de fechar).
pub fn fechar(address: &str) -> Result<(), String> {
    if !address_ok(address) { return Err("Janela inválida".into()); }
    hypr(&["dispatch", "closewindow", &format!("address:{address}")]).map(|_| ())
}

// ---------------------------------------------------------------------------------------------
// Teclado

/// Nome de tecla dito pela pessoa → nome XKB do `wtype` ("enter" → "Return").
fn tecla(name: &str) -> Option<String> {
    let lower = name.trim().to_lowercase();
    let named = match lower.as_str() {
        "enter" | "return" => "Return", "esc" | "escape" => "Escape", "tab" => "Tab", "espaco" | "espaço" | "space" => "space",
        "backspace" | "apagar" => "BackSpace", "delete" | "del" => "Delete", "insert" => "Insert",
        "home" | "inicio" | "início" => "Home", "end" | "fim" => "End", "pageup" | "pgup" => "Prior", "pagedown" | "pgdn" => "Next",
        "up" | "cima" => "Up", "down" | "baixo" => "Down", "left" | "esquerda" => "Left", "right" | "direita" => "Right",
        "print" | "printscreen" => "Print",
        _ => "",
    };
    if !named.is_empty() { return Some(named.to_string()); }
    if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) { return (1..=24).contains(&n).then(|| format!("F{n}")); }
    let mut chars = lower.chars();
    match (chars.next(), chars.next()) { (Some(c), None) if c.is_ascii_alphanumeric() || "-=[];',./`\\".contains(c) => Some(c.to_string()), _ => None }
}

/// "ctrl+shift+t" → argumentos do `wtype` (aperta os modificadores, a tecla e solta).
pub fn combo_args(combo: &str) -> Result<Vec<String>, String> {
    let parts: Vec<&str> = combo.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    let Some((last, mods)) = parts.split_last() else { return Err("Diga a tecla (ex.: ctrl+c, enter, alt+tab)".into()) };
    let mut args = Vec::new();
    let mut held = Vec::new();
    for m in mods {
        let name = match m.to_lowercase().as_str() {
            "ctrl" | "control" | "controle" => "ctrl", "shift" => "shift", "alt" => "alt", "altgr" => "altgr",
            "super" | "win" | "meta" | "logo" => "logo",
            other => return Err(format!("Modificador desconhecido: {other}")),
        };
        if !held.contains(&name) { args.extend(["-M".to_string(), name.to_string()]); held.push(name); }
    }
    let key = tecla(last).ok_or_else(|| format!("Tecla desconhecida: {last}"))?;
    args.extend(["-k".to_string(), key]);
    for name in held.iter().rev() { args.extend(["-m".to_string(), name.to_string()]); }
    Ok(args)
}

fn wtype(args: &[String]) -> Result<(), String> {
    let ok = Command::new("wtype").args(args).stdout(Stdio::null()).stderr(Stdio::null()).status()
        .map_err(|_| "Para usar o teclado, instale o wtype")?.success();
    if ok { Ok(()) } else { Err("O teclado virtual foi recusado (o compositor precisa do protocolo virtual-keyboard)".into()) }
}

/// Digita o texto na janela em foco (acentos inclusive).
pub fn digitar(texto: &str) -> Result<(), String> {
    if texto.is_empty() { return Ok(()); }
    if texto.chars().count() > 5000 { return Err("Texto longo demais (até 5.000 caracteres)".into()); }
    // "--" para um texto que comece com "-" não virar opção do wtype.
    wtype(&["-d".to_string(), "4".to_string(), "--".to_string(), texto.to_string()])
}

pub fn teclas(combo: &str) -> Result<(), String> {
    wtype(&combo_args(combo)?)
}

// ---------------------------------------------------------------------------------------------
// Mouse

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Botao { Esquerdo, Direito, Meio }

/// Código do `ydotool click` (aperta e solta).
pub fn click_code(botao: Botao) -> &'static str {
    match botao { Botao::Esquerdo => "0xC0", Botao::Direito => "0xC1", Botao::Meio => "0xC2" }
}

/// O `ydotoold` da Ayla: socket privado nos dados dela; um só por vez.
static DAEMON: Mutex<Option<(Child, PathBuf)>> = Mutex::new(None);

fn pid_file(dir: &Path) -> PathBuf { dir.join("ydotoold.pid") }

/// Um `ydotoold` que sobrou de uma Ayla que caiu: encerra (só se for mesmo o nosso).
fn reap_stale(dir: &Path, socket: &Path) {
    let Ok(pid) = std::fs::read_to_string(pid_file(dir)) else { return };
    let pid = pid.trim();
    if pid.parse::<u32>().is_err() { return; }
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
    let cmdline = String::from_utf8_lossy(&cmdline);
    if cmdline.contains("ydotoold") && cmdline.contains(&*socket.to_string_lossy()) {
        let _ = Command::new("kill").arg(pid).status();
    }
    let _ = std::fs::remove_file(pid_file(dir));
}

fn socket_of(dir: &Path) -> Result<PathBuf, String> {
    let mut daemon = DAEMON.lock().map_err(|_| "O mouse está ocupado")?;
    if let Some((child, socket)) = daemon.as_mut() {
        if child.try_wait().ok().flatten().is_none() && socket.exists() { return Ok(socket.clone()); }
    }
    std::fs::create_dir_all(dir).map_err(|_| "Não foi possível preparar o mouse")?;
    let socket = dir.join("ydotool.sock");
    reap_stale(dir, &socket);
    let _ = std::fs::remove_file(&socket);
    let child = Command::new("ydotoold").arg(format!("--socket-path={}", socket.display())).arg("--socket-perm=0600")
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
        .map_err(|_| "Para clicar e rolar, instale o ydotool")?;
    let _ = std::fs::write(pid_file(dir), child.id().to_string());
    // O dispositivo virtual leva um instante para o compositor reconhecer.
    for _ in 0..30 { if socket.exists() { break; } std::thread::sleep(Duration::from_millis(50)); }
    std::thread::sleep(Duration::from_millis(250));
    if !socket.exists() { return Err("O mouse virtual não iniciou (o ydotoold precisa acessar /dev/uinput)".into()); }
    *daemon = Some((child, socket.clone()));
    Ok(socket)
}

/// Encerra o `ydotoold` da Ayla (ao fechar).
pub fn parar_mouse(dir: &Path) {
    if let Ok(mut daemon) = DAEMON.lock() {
        if let Some((mut child, socket)) = daemon.take() { let _ = child.kill(); let _ = child.wait(); let _ = std::fs::remove_file(socket); }
    }
    let _ = std::fs::remove_file(pid_file(dir));
}

fn ydotool(dir: &Path, args: &[&str]) -> Result<(), String> {
    let socket = socket_of(dir)?;
    let ok = Command::new("ydotool").args(args).env("YDOTOOL_SOCKET", &socket).stdout(Stdio::null()).stderr(Stdio::null()).status()
        .map_err(|_| "Para clicar e rolar, instale o ydotool")?.success();
    if ok { Ok(()) } else { Err("O mouse virtual recusou".into()) }
}

/// Leva o cursor ao ponto (coordenadas da área de trabalho inteira, como o Hyprland).
pub fn mover(x: i64, y: i64) -> Result<(), String> {
    if !(-20_000..=20_000).contains(&x) || !(-20_000..=20_000).contains(&y) { return Err("Posição fora da tela".into()); }
    hypr(&["dispatch", "movecursor", &x.to_string(), &y.to_string()]).map(|_| ())
}

/// Clica (no ponto, se dado). `vezes` 2 = clique duplo.
pub fn clicar(dir: &Path, botao: Botao, ponto: Option<(i64, i64)>, vezes: u8) -> Result<(), String> {
    if let Some((x, y)) = ponto { mover(x, y)?; std::thread::sleep(Duration::from_millis(60)); }
    let repeat = vezes.clamp(1, 3).to_string();
    ydotool(dir, &["click", "--repeat", &repeat, "--next-delay", "60", click_code(botao)])
}

/// Aperta e solta teclas pelos códigos físicos do Linux (macros gravadas): o resultado segue o
/// layout do sistema, como no teclado de verdade.
pub fn teclas_brutas(dir: &Path, teclas: &[(u16, bool)]) -> Result<(), String> {
    for chunk in teclas.chunks(120) {
        let args: Vec<String> = chunk.iter().map(|(c, down)| format!("{c}:{}", u8::from(*down))).collect();
        let mut all = vec!["key".to_string(), "--key-delay".to_string(), "8".to_string()];
        all.extend(args);
        let refs: Vec<&str> = all.iter().map(String::as_str).collect();
        ydotool(dir, &refs)?;
    }
    Ok(())
}

/// Arrasta: aperta no início, leva o cursor em passos até o fim e solta.
pub fn arrastar(dir: &Path, botao: Botao, de: (i64, i64), para: (i64, i64)) -> Result<(), String> {
    let base = match botao { Botao::Esquerdo => 0x00, Botao::Direito => 0x01, Botao::Meio => 0x02 };
    mover(de.0, de.1)?;
    std::thread::sleep(Duration::from_millis(60));
    ydotool(dir, &["click", &format!("0x{:02X}", 0x40 | base)])?;
    for i in 1..=12 {
        let x = de.0 + (para.0 - de.0) * i / 12;
        let y = de.1 + (para.1 - de.1) * i / 12;
        let _ = mover(x, y);
        std::thread::sleep(Duration::from_millis(16));
    }
    ydotool(dir, &["click", &format!("0x{:02X}", 0x80 | base)])
}

/// Rola: positivo desce, negativo sobe (em "dentes" da roda).
pub fn rolar(dir: &Path, dentes: i32) -> Result<(), String> {
    let steps = dentes.clamp(-50, 50);
    if steps == 0 { return Ok(()); }
    // No ydotool, a roda para baixo é y negativo.
    ydotool(dir, &["mousemove", "--wheel", "-x", "0", "-y", &(-steps).to_string()])
}

/// Posição do cursor agora.
pub fn cursor() -> Result<(i64, i64), String> {
    let text = hypr(&["cursorpos"])?;
    let (x, y) = text.split_once(',').ok_or("Resposta inválida do Hyprland")?;
    Ok((x.trim().parse().map_err(|_| "Posição inválida")?, y.trim().parse().map_err(|_| "Posição inválida")?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atalhos_viram_argumentos_do_wtype() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(combo_args("ctrl+c").unwrap(), s(&["-M", "ctrl", "-k", "c", "-m", "ctrl"]));
        assert_eq!(combo_args("Ctrl + Shift + T").unwrap(), s(&["-M", "ctrl", "-M", "shift", "-k", "t", "-m", "shift", "-m", "ctrl"]));
        assert_eq!(combo_args("enter").unwrap(), s(&["-k", "Return"]));
        assert_eq!(combo_args("super+espaço").unwrap(), s(&["-M", "logo", "-k", "space", "-m", "logo"]));
        assert_eq!(combo_args("alt+f4").unwrap(), s(&["-M", "alt", "-k", "F4", "-m", "alt"]));
        assert!(combo_args("ctrl+").is_err() && combo_args("hiper+x").is_err() && combo_args("ctrl+banana").is_err());
    }

    #[test]
    fn botoes_janelas_e_limites() {
        assert_eq!((click_code(Botao::Esquerdo), click_code(Botao::Direito), click_code(Botao::Meio)), ("0xC0", "0xC1", "0xC2"));
        assert!(address_ok("0x55d1c0a3b2f0") && !address_ok("0xZZ") && !address_ok("55d1; rm -rf") && !address_ok("0x1 address:0x2"));
        assert!(mover(99_999, 0).is_err());
        assert!(digitar(&"a".repeat(5001)).is_err());
    }
}
