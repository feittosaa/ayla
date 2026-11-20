//! Integração Linux. A janela e o WebView são os mesmos nos dois modos.
use crate::core::{preferences::{hex_color, PreferenceStore}, services::Core};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, path::{Path, PathBuf}, process::Command, sync::{Mutex, mpsc}};
use tauri::{Emitter, Manager, AppHandle, WebviewWindow};
use webkit2gtk::gio::{self, prelude::*};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    App,
    Quick,
    Login,
}
impl Launch {
    pub fn from_args() -> Self {
        match std::env::args().nth(1).as_deref() {
            Some("--quick") => Self::Quick,
            Some("--background") => Self::Login,
            _ => Self::App,
        }
    }
}

fn xdg(var: &str, relative: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(relative))
}

// GApplication encaminha as ativações à instância dona do nome no D-Bus da sessão.
pub fn register(launch: Launch) -> Result<Option<(gio::Application, mpsc::Receiver<Launch>)>, String> {
    let root = std::env::var_os("AYLA_DATA_DIR").map(PathBuf::from)
        .unwrap_or_else(|| xdg("XDG_DATA_HOME", ".local/share").join("dev.feit.ayla"));
    if launch == Launch::Login && !PreferenceStore::new(&root).load().map_err(|e| e.to_string())?.desktop.autostart {
        return Ok(None);
    }
    let path = std::path::absolute(root).map_err(|e| e.to_string())?;
    let digest = format!("{:x}", Sha256::digest(path.as_os_str().as_encoded_bytes()));
    let id = format!("dev.feit.ayla.Session.p{}", &digest[..20]);
    let instance = gio::Application::new(Some(&id), gio::ApplicationFlags::empty());
    let (tx, rx) = mpsc::channel();
    for (name, mode) in [("show", Launch::App), ("quick", Launch::Quick)] {
        let action = gio::SimpleAction::new(name, None);
        let sender = tx.clone();
        action.connect_activate(move |_, _| { let _ = sender.send(mode); });
        instance.add_action(&action);
    }
    instance.register(gio::Cancellable::NONE).map_err(|e| e.to_string())?;
    if instance.is_remote() {
        if launch != Launch::Login {
            instance.activate_action(if launch == Launch::Quick { "quick" } else { "show" }, None);
            if let Some(bus) = instance.dbus_connection() { bus.flush_sync(gio::Cancellable::NONE).map_err(|e| e.to_string())?; }
        }
        return Ok(None);
    }
    Ok(Some((instance, rx)))
}

#[derive(Default)]
pub struct View {
    quick: bool,
    first_opened: bool,
    normal_size: Option<tauri::PhysicalSize<u32>>,
    normal_position: Option<tauri::PhysicalPosition<i32>>,
    normal_maximized: bool,
    generation: u64,
    pending_show: Option<u64>,
    dialogs: usize,
}

impl View {
    pub fn cancel_placement(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.pending_show = None;
    }
}

fn hypr_json(command: &str) -> Option<Value> {
    let out = Command::new("hyprctl").args(["-j", command]).output().ok()?;
    out.status.success().then(|| serde_json::from_slice(&out.stdout).ok()).flatten()
}
fn hypr_batch(commands: &str) -> bool {
    Command::new("hyprctl").args(["--batch", commands]).output()
        .is_ok_and(|out| out.status.success() && String::from_utf8_lossy(&out.stdout).lines().all(|line| line.trim().trim_end_matches("ok").is_empty()))
}
fn is_hypr() -> bool { std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() }
const NORMAL_SELECTOR: &str = r"initialclass:^(ayla|app|dev\.feit\.ayla)$,initialtitle:^(Ayla)$";

fn panel_title() -> String {
    format!("Ayla · Visão rápida · {}", std::process::id())
}

fn panel_rules(monitor: &Value, radius: u8) -> Option<Vec<String>> {
    let scale = monitor["scale"].as_f64()?;
    if scale <= 0.0 { return None; }
    let (mut width, mut height) = (monitor["width"].as_i64()?, monitor["height"].as_i64()?);
    if monitor["transform"].as_i64().unwrap_or(0) % 2 != 0 {
        std::mem::swap(&mut width, &mut height);
    }
    // A regra estática move usa coordenadas lógicas relativas ao monitor escolhido.
    let inset = |i: usize| monitor["reserved"][i].as_i64().unwrap_or(0) + 5;
    let width = ((width as f64 / scale) as i64 - inset(0) - inset(2)).clamp(1, 460);
    let height = ((height as f64 / scale) as i64 - inset(1) - inset(3)).max(1);
    let id = monitor["id"].as_i64()?;
    Some(vec![
        "float".into(), format!("monitor {id}"), format!("size {width} {height}"),
        format!("move {} {}", inset(0), inset(1)), "bordersize 0".into(),
        "noshadow on".into(), format!("rounding {radius}"), "animation slide left".into(),
    ])
}

fn configure_hypr_rules(selector: &str, rules: &[&str]) -> bool {
    let commands = std::iter::once("unset").chain(rules.iter().copied())
        .map(|rule| format!("keyword windowrule {rule}, {selector}"))
        .collect::<Vec<_>>().join("; ");
    hypr_batch(&commands)
}

fn configure_hypr_panel(rules: &[String]) -> bool {
    let selector = format!("title:^({})$", panel_title());
    let rules: Vec<_> = rules.iter().map(String::as_str).collect();
    configure_hypr_rules(&selector, &rules)
}

fn needs_tiling(clients: &Value, pid: u32) -> bool {
    let Some(clients) = clients.as_array() else { return false };
    clients.iter().any(|client| {
        client["pid"].as_u64() == Some(u64::from(pid))
            && client["title"] == "Ayla"
            && client["mapped"] == true
            && (client["floating"] == true
                || client["fullscreen"].as_i64().unwrap_or(0) != 0
                || client["fullscreenClient"].as_i64().unwrap_or(0) != 0)
    })
}

pub fn clear_rules() {
    if is_hypr() {
        configure_hypr_panel(&[]);
        configure_hypr_rules(NORMAL_SELECTOR, &[]);
    }
}

extern "C" {
    fn ayla_panel_click_away(window: *mut std::ffi::c_void, armed: i32);
}

// GTK e sua conexão Wayland pertencem à thread da interface.
fn click_away(window: &WebviewWindow, armed: bool) -> Result<(), String> {
    use webkit2gtk::glib::object::ObjectType;
    let gtk = window.gtk_window().map_err(|e| e.to_string())?;
    unsafe { ayla_panel_click_away(gtk.as_ptr().cast(), i32::from(armed)); }
    Ok(())
}

async fn dialog_focus(app: &AppHandle, opening: bool) -> Result<(), String> {
    let handle = app.clone();
    let (done, finished) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let state = handle.state::<Mutex<View>>();
        let mut view = state.lock().unwrap();
        if opening { view.dialogs += 1; } else { view.dialogs = view.dialogs.saturating_sub(1); }
        let armed = view.quick && view.dialogs == 0 && view.pending_show.is_none();
        drop(view);
        if let Some(window) = handle.get_webview_window("main") {
            let _ = click_away(&window, armed && window.is_visible().unwrap_or(false));
        }
        let _ = done.send(());
    }).map_err(|e| e.to_string())?;
    finished.await.map_err(|_| "Janela indisponível".to_string())
}

pub async fn panel_dialog<T: Send + 'static>(
    app: &AppHandle, work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    dialog_focus(app, true).await?;
    let result = tauri::async_runtime::spawn_blocking(work).await;
    dialog_focus(app, false).await?;
    result.map_err(|_| "Não foi possível abrir o seletor de arquivos".into())
}

fn material_palette(path: &Path) -> Value {
    let data: Value = fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let mut colors = serde_json::Map::new();
    for role in ["background","surface_container","surface_container_high","surface_container_low","surface_container_highest","on_surface","on_surface_variant","outline_variant","outline","primary","on_primary","primary_container","error"] {
        if let Some(v) = data[role].as_str().filter(|v| hex_color(v)) { colors.insert(role.to_owned(), json!(v)); }
    }
    Value::Object(colors)
}
fn qml_value<'a>(qml: &'a str, declaration: &str) -> Option<&'a str> {
    qml.lines().find_map(|line| line.trim().strip_prefix(declaration)).map(|s| s.split("//").next().unwrap_or("").trim())
}
fn system_theme() -> Value {
    let colors = material_palette(&xdg("XDG_STATE_HOME",".local/state").join("quickshell/user/generated/colors.json"));
    let qml = fs::read_to_string(xdg("XDG_CONFIG_HOME",".config").join("quickshell/ii/modules/common/Appearance.qml")).unwrap_or_default();
    let mut font = qml_value(&qml, "property string main:").map(|s| s.trim_matches('"').to_owned());
    if font.is_none() {
        font = Command::new("gsettings").args(["get","org.gnome.desktop.interface","font-name"]).output().ok()
            .filter(|o| o.status.success()).and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|v| v.trim().trim_matches('\'').trim_end_matches(|c:char| c.is_ascii_digit() || c=='.').trim_end().to_owned());
    }
    let number = |name: &str| qml_value(&qml, name).and_then(|s| s.parse::<u8>().ok());
    let corner = qml_value(&qml, "property int screenRounding:").and_then(|v| {
        v.parse::<u8>().ok().or_else(|| number(&format!("property int {v}:")))
    });
    let radius = corner.map(|r| r.saturating_sub(number("property real hyprlandGapsOut:").unwrap_or(5)).saturating_sub(1))
        .or_else(|| number("property int windowRounding:")).unwrap_or(18).min(48);
    json!({"colors":colors,"font":font,"radius":radius})
}

#[tauri::command]
pub fn desktop_view(app: AppHandle) -> Value {
    let (quick, pending) = {
        let state = app.state::<Mutex<View>>();
        let view = state.lock().unwrap();
        (view.quick, view.pending_show)
    };
    json!({"quick":quick, "pending":pending, "theme":if quick {system_theme()} else {Value::Null}})
}

fn size_compact_window(window: &WebviewWindow) -> tauri::Result<()> {
    let pointer_monitor = window.cursor_position().ok()
        .and_then(|p| window.monitor_from_point(p.x, p.y).ok().flatten());
    if let Some(monitor) = pointer_monitor.or(window.current_monitor()?) {
        let scale = monitor.scale_factor();
        let size = monitor.size().to_logical::<f64>(scale);
        let pos = monitor.position();
        let gap = (5.0 * scale) as i32;
        window.set_size(tauri::LogicalSize::new(size.width.min(460.0), (size.height - 30.0).max(1.0)))?;
        // Alguns compositores Wayland escolhem a posição das janelas comuns.
        let _ = window.set_position(tauri::PhysicalPosition::new(pos.x + gap, pos.y + gap));
    }
    Ok(())
}

fn set_mode(app: &AppHandle, quick: bool) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("Janela indisponível")?;
    let hypr = is_hypr();
    // Ler antes de mostrar a Ayla: o monitor em foco é o da pessoa que abriu a lateral.
    let state = app.state::<Mutex<View>>();
    let mut view = state.lock().unwrap();
    view.cancel_placement();
    let previous = view.quick;
    if quick && !previous && !hypr {
        view.normal_size = window.inner_size().ok();
        view.normal_position = window.outer_position().ok();
        view.normal_maximized = window.is_maximized().unwrap_or(false);
    }
    let (size, position, maximized) = (view.normal_size, view.normal_position, view.normal_maximized);
    drop(view);

    let tile = hypr && !quick && (previous
        || !window.is_visible().unwrap_or(false)
        || hypr_json("clients").is_some_and(|clients| needs_tiling(&clients, std::process::id())));
    if tile && !configure_hypr_rules(NORMAL_SELECTOR, &["tile", "fullscreenstate 0 0"]) {
        eprintln!("O Hyprland não aceitou as regras da janela normal da Ayla");
    }

    if quick || previous || tile {
        click_away(&window, false)?;
        let rules = if quick && hypr {
            let prefs = app.state::<Core>().prefs()?.desktop;
            let radius = if prefs.system_theme {
                system_theme()["radius"].as_u64().unwrap_or(18) as u8
            } else { prefs.panel_radius };
            let monitor = hypr_json("monitors").and_then(|items| items.as_array()?.iter().find(|m| m["focused"] == true).cloned())
                .ok_or("Não foi possível identificar o monitor ativo")?;
            Some(panel_rules(&monitor, radius).ok_or("Geometria de monitor inválida")?)
        } else { None };
        if let Some(rules) = &rules {
            if !configure_hypr_panel(rules) { return Err("O Hyprland não aceitou as regras da visão rápida".into()); }
        }
        if tile || window.is_fullscreen().map_err(|e| e.to_string())? {
            window.set_fullscreen(false).map_err(|e| e.to_string())?;
        }
        if tile || window.is_maximized().map_err(|e| e.to_string())? {
            window.unmaximize().map_err(|e| e.to_string())?;
        }
        // Reapresentar a mesma superfície faz o compositor aplicar as regras ANTES do primeiro quadro.
        window.hide().map_err(|e| e.to_string())?;
        let configure = || -> tauri::Result<()> {
            window.set_title(if quick { panel_title() } else { "Ayla".into() }.as_str())?;
            window.set_decorations(!quick)?;
            window.set_always_on_top(quick)?;
            if quick && rules.is_none() { size_compact_window(&window)?; }
            if !quick && !hypr {
                if let Some(size) = size { window.set_size(size)?; }
                if let Some(position) = position { let _ = window.set_position(position); }
                if maximized { window.maximize()?; }
            }
            Ok(())
        };
        configure().map_err(|e| e.to_string())?;
    }

    let mut view = state.lock().unwrap();
    view.quick = quick;
    if quick { view.pending_show = Some(view.generation); }
    drop(view);
    let _ = app.emit("desktop-view-changed", quick);
    if quick { return Ok(()); }
    window.show().map_err(|e| e.to_string())?;
    window.unminimize().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    prepare_on_first_open(app);
    Ok(())
}

#[tauri::command]
pub fn quick_ready(app: AppHandle, generation: u64) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("Janela indisponível")?;
    let state = app.state::<Mutex<View>>();
    let mut view = state.lock().unwrap();
    if view.pending_show != Some(generation) { return Ok(()); }
    view.pending_show = None;
    let armed = view.dialogs == 0;
    drop(view);
    window.show().map_err(|e| e.to_string())?;
    window.unminimize().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    click_away(&window, armed)?;
    prepare_on_first_open(&app);
    Ok(())
}

fn prepare_on_first_open(app: &AppHandle) {
    let state = app.state::<Mutex<View>>();
    let first_open = !std::mem::replace(&mut state.lock().unwrap().first_opened, true);
    if first_open {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = handle.state::<Core>().prepare_defaults(&handle).await;
        });
    }
}

pub fn launch(app: &AppHandle, mode: Launch) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("Janela indisponível")?;
    let (quick, pending) = {
        let state = app.state::<Mutex<View>>();
        let view = state.lock().unwrap();
        (view.quick, view.pending_show.is_some())
    };
    match mode {
        Launch::Login if app.state::<Core>().prefs()?.desktop.background => Ok(()),
        Launch::Quick if quick && (pending || window.is_visible().unwrap_or(false)) => close(app, &window),
        _ => set_mode(app, mode == Launch::Quick),
    }
}

fn close(app: &AppHandle, window: &WebviewWindow) -> Result<(), String> {
    click_away(window, false)?;
    let background = app.state::<Core>().prefs()?.desktop.background;
    app.state::<Mutex<View>>().lock().unwrap().cancel_placement();
    window.hide().map_err(|e| e.to_string())?;
    if !background { app.exit(0); }
    Ok(())
}

#[tauri::command]
pub fn desktop_action(app: AppHandle, action: String) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("Janela indisponível")?;
    match action.as_str() {
        "open" => launch(&app, Launch::App),
        "quick" => launch(&app, Launch::Quick),
        "hide" => close(&app, &window),
        "restore" => window.set_fullscreen(false).map_err(|e| e.to_string()),
        "quit" => { app.exit(0); Ok(()) }
        _ => Err("Ação de janela desconhecida".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn positions_the_panel_on_the_focused_scaled_rotated_output() {
        let m=json!({"id":3,"x":-900,"y":70,"width":1800,"height":1200,"scale":1.5,"transform":1,"reserved":[18,30,0,12],"activeWorkspace":{"id":4}});
        let rules = panel_rules(&m, 17).unwrap();
        assert!(rules.iter().any(|r| r == "size 460 1148"));
        assert!(rules.iter().any(|r| r == "move 23 35"));
        assert!(rules.iter().any(|r| r == "monitor 3"));
    }

    #[test]
    fn only_retiles_the_mapped_ayla_window_if_the_compositor_reports_a_non_tile_state() {
        let mut ayla = json!({"pid":42,"title":"Ayla","mapped":true,"floating":false,"fullscreen":0,"fullscreenClient":0});
        let other = json!({"pid":99,"title":"Ayla","mapped":true,"floating":false,"fullscreen":2});
        assert!(!needs_tiling(&json!([&ayla, &other]), 42));
        for (key, value) in [("floating", json!(true)), ("fullscreen", json!(1)), ("fullscreenClient", json!(2))] {
            let previous = std::mem::replace(&mut ayla[key], value);
            assert!(needs_tiling(&json!([&ayla, &other]), 42), "{key}");
            ayla[key] = previous;
        }
    }
}
