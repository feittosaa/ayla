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
    /// Athena sozinha, como bloco de notas (com os arquivos da fila `ATHENA_QUEUE`).
    Athena,
}
impl Launch {
    pub fn from_args() -> Self {
        let mut args = std::env::args().skip(1);
        match args.next().as_deref() {
            Some("--quick") => Self::Quick,
            Some("--background") => Self::Login,
            Some("--abrir") if args.next().as_deref() == Some("athena") => Self::Athena,
            _ => Self::App,
        }
    }
}

/// "Abrir com" do gerenciador de arquivos: `ayla --abrir <ayla|athena|iris> <arquivos…>`.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OpenRequest {
    pub target: String,
    pub paths: Vec<String>,
}
/// Pedidos ainda não lidos pela interface (ela os busca ao montar e a cada aviso `open-files`).
static OPEN_QUEUE: Mutex<Vec<OpenRequest>> = Mutex::new(Vec::new());
/// Arquivos para a janela da Athena; ela os busca ao montar e a cada aviso `athena-open`.
static ATHENA_QUEUE: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Entrega (uma vez) os arquivos pedidos à Athena.
pub fn athena_requests() -> Vec<String> {
    std::mem::take(&mut *ATHENA_QUEUE.lock().unwrap())
}
/// Guarda o pedido na fila certa e diz como abrir: a Athena tem janela própria.
fn enqueue(request: OpenRequest) -> Launch {
    if request.target == "athena" {
        ATHENA_QUEUE.lock().unwrap().extend(request.paths);
        Launch::Athena
    } else {
        OPEN_QUEUE.lock().unwrap().push(request);
        Launch::App
    }
}
pub const ATHENA_WINDOW: &str = "athena";

/// Mostra (ou cria) a janela da Athena e avisa que há arquivos na fila.
pub fn show_athena(app: &AppHandle) -> Result<(), String> {
    let window = match app.get_webview_window(ATHENA_WINDOW) {
        Some(window) => window,
        None => {
            let window = tauri::WebviewWindowBuilder::new(app, ATHENA_WINDOW, tauri::WebviewUrl::App("index.html".into()))
                .title("Athena")
                .inner_size(900.0, 720.0)
                .min_inner_size(360.0, 320.0)
                // Sem a barra do sistema: o header da Athena faz esse papel, como no GNOME Text Editor.
                .decorations(false)
                .build()
                .map_err(|e| e.to_string())?;
            // Arquivos arrastados para a janela abrem em abas (liberados aqui, pelo evento do sistema).
            let handle = app.clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = event {
                    let files: Vec<String> = paths.iter().filter(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned()).collect();
                    if files.is_empty() { return; }
                    ATHENA_QUEUE.lock().unwrap().extend(files);
                    let _ = handle.emit_to(ATHENA_WINDOW, "athena-open", ());
                }
            });
            window
        }
    };
    // Corretor ortográfico do WebKitGTK (dicionários do sistema; os ausentes são ignorados).
    let _ = window.with_webview(|webview| {
        use webkit2gtk::{WebContextExt, WebViewExt};
        if let Some(context) = webview.inner().context() {
            context.set_spell_checking_enabled(true);
            context.set_spell_checking_languages(&["pt_BR", "pt_PT", "en_US"]);
        }
    });
    window.show().map_err(|e| e.to_string())?;
    window.unminimize().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    if !ATHENA_QUEUE.lock().unwrap().is_empty() {
        app.emit_to(ATHENA_WINDOW, "athena-open", ()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

impl OpenRequest {
    pub fn from_args() -> Option<Self> {
        Self::parse(std::env::args().skip(1))
    }
    /// Caminhos absolutos, resolvidos aqui: o processo que recebe o pedido tem outra pasta atual.
    fn parse(mut args: impl Iterator<Item = String>) -> Option<Self> {
        if args.next().as_deref() != Some("--abrir") {
            return None;
        }
        let target = args.next().filter(|t| matches!(t.as_str(), "ayla" | "athena" | "iris"))?;
        let paths: Vec<String> = args.filter_map(|arg| gio::File::for_commandline_arg(arg).path())
            .map(|p| p.to_string_lossy().into_owned()).collect();
        // A Athena abre também sem arquivos (pelo menu), como um bloco de notas.
        (!paths.is_empty() || target == "athena").then_some(Self { target, paths })
    }
}

/// Entrega os pedidos de abertura pendentes (cada um uma única vez).
#[tauri::command]
pub fn open_requests() -> Vec<OpenRequest> {
    std::mem::take(&mut *OPEN_QUEUE.lock().unwrap())
}

fn xdg(var: &str, relative: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(relative))
}

/// Pasta dos ícones da Ayla e das entradas do menu (os mesmos caminhos que o instalador usa).
/// `None` num perfil separado (`AYLA_DATA_DIR`), para testes não trocarem o ícone real do usuário.
fn system_dirs() -> Option<(PathBuf, PathBuf, PathBuf)> {
    if std::env::var_os("AYLA_DATA_DIR").is_some() {
        return None;
    }
    let data = xdg("XDG_DATA_HOME", ".local/share");
    Some((data.join("icons/hicolor/512x512/apps"), data.join("applications"), xdg("XDG_CONFIG_HOME", ".config").join("autostart")))
}

/// Valida o PNG do ícone (base64, até 2 MB, 512×512) e devolve os bytes e os pixels RGBA.
fn decode_icon(png: &str) -> Result<(Vec<u8>, Vec<u8>), String> {
    use base64::Engine;
    use gdk_pixbuf::prelude::PixbufLoaderExt;
    let bytes = base64::engine::general_purpose::STANDARD.decode(png).map_err(|_| "Ícone inválido")?;
    if bytes.len() > 2 << 20 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("Ícone inválido".into());
    }
    let loader = gdk_pixbuf::PixbufLoader::with_type("png").map_err(|_| "Ícone inválido")?;
    loader.write(&bytes).and_then(|()| loader.close()).map_err(|_| "Ícone inválido")?;
    let pixbuf = loader.pixbuf().ok_or("Ícone inválido")?.add_alpha(false, 0, 0, 0).map_err(|_| "Ícone inválido")?;
    if pixbuf.width() != ICON_SIDE || pixbuf.height() != ICON_SIDE {
        return Err("O ícone deve ter 512×512".into());
    }
    let (stride, side) = (pixbuf.rowstride() as usize, ICON_SIDE as usize);
    let raw = pixbuf.read_pixel_bytes();
    let rgba = (0..side).flat_map(|row| raw[row * stride..row * stride + side * 4].iter().copied()).collect();
    Ok((bytes, rgba))
}
const ICON_SIDE: i32 = 512;

/// Ícones escolhidos em Aparência: PNGs 512×512 desenhados pela interface. O da Ayla vira o
/// ícone da janela; os três vão para o menu e o "Abrir com" (Ayla, Athena e Iris).
#[tauri::command]
pub fn appearance_icon(window: WebviewWindow, png: String, athena: Option<String>, iris: Option<String>) -> Result<(), String> {
    let (bytes, rgba) = decode_icon(&png)?;
    let side = ICON_SIDE as u32;
    window.set_icon(tauri::image::Image::new_owned(rgba, side, side)).map_err(|_| "Não foi possível trocar o ícone da janela")?;
    let Some((icons, applications, autostart)) = system_dirs() else { return Ok(()) };
    let mut changed = false;
    let mut install = |base: &str, bytes: &[u8], entries: &[PathBuf]| -> Result<(), String> {
        let (name, fresh) = install_icon(&icons, base, bytes)?;
        changed |= fresh;
        // Caminho absoluto: barras e menus leem o arquivo direto, sem depender do cache do tema.
        let path = icons.join(format!("{name}.png"));
        for entry in entries {
            changed |= set_entry_icon(entry, &path.to_string_lossy())?;
        }
        Ok(())
    };
    install(ICON_BASE, &bytes, &[applications.join("dev.feit.ayla.desktop"), applications.join("dev.feit.ayla-login.desktop"), autostart.join("dev.feit.ayla.desktop")])?;
    for (organ, png) in [("athena", athena), ("iris", iris)] {
        let Some(png) = png else { continue };
        let (bytes, _) = decode_icon(&png)?;
        install(&format!("{ICON_BASE}.{organ}"), &bytes, &[applications.join(format!("{ICON_BASE}.{organ}.desktop"))])?;
    }
    if changed {
        refresh_icon_cache(&icons);
    }
    Ok(())
}
const ICON_BASE: &str = "dev.feit.ayla";

/// Grava o ícone com um nome que muda junto com o conteúdo (`<base>-<hash>.png`): barras e menus
/// guardam ícones pelo nome, então um nome novo é a forma segura de trocar o desenho sem
/// reiniciá-los. Remove as versões anteriores. Devolve o nome e se algo mudou no disco.
fn install_icon(dir: &Path, base: &str, bytes: &[u8]) -> Result<(String, bool), String> {
    let name = format!("{base}-{}", &format!("{:x}", Sha256::digest(bytes))[..8]);
    let path = dir.join(format!("{name}.png"));
    let mut changed = false;
    if fs::read(&path).ok().as_deref() != Some(bytes) {
        fs::create_dir_all(dir).map_err(|_| "Não foi possível guardar o ícone")?;
        let temporary = dir.join(format!(".{name}.png.tmp"));
        fs::write(&temporary, bytes).and_then(|()| fs::rename(&temporary, &path)).map_err(|_| "Não foi possível guardar o ícone")?;
        changed = true;
    }
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        let old = file.strip_suffix(".png").and_then(|f| f.strip_prefix(base)).and_then(|f| f.strip_prefix('-'))
            .is_some_and(|hash| hash.len() == 8 && hash.bytes().all(|b| b.is_ascii_hexdigit()));
        if old && file != format!("{name}.png") {
            changed |= fs::remove_file(entry.path()).is_ok();
        }
    }
    Ok((name, changed))
}

/// Troca o `Icon=` de uma entrada instalada (se existir). Devolve se mudou.
fn set_entry_icon(entry: &Path, icon: &str) -> Result<bool, String> {
    let Ok(text) = fs::read_to_string(entry) else { return Ok(false) };
    let updated: String = text.lines().map(|line| if line.starts_with("Icon=") { format!("Icon={icon}\n") } else { format!("{line}\n") }).collect();
    if updated == text {
        return Ok(false);
    }
    let temporary = entry.with_extension("desktop.tmp");
    fs::write(&temporary, updated).and_then(|()| fs::rename(&temporary, entry)).map_err(|_| "Não foi possível atualizar o menu de aplicativos")?;
    Ok(true)
}

/// Um `icon-theme.cache` antigo na pasta de ícones do usuário (criado por outros programas)
/// esconde dos menus e da barra os ícones que não estavam nele. Avisa que a pasta mudou e refaz
/// o cache; sem a ferramenta, remove-o (os menus voltam a ler a pasta).
fn refresh_icon_cache(icons: &Path) {
    let Some(theme) = icons.ancestors().nth(2) else { return };
    let _ = fs::File::open(theme).and_then(|f| f.set_modified(std::time::SystemTime::now()));
    let cache = theme.join("icon-theme.cache");
    if cache.exists() {
        let rebuilt = Command::new("gtk-update-icon-cache").args(["-q", "-f", "-t"]).arg(theme)
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
        if !rebuilt {
            let _ = fs::remove_file(&cache);
        }
    }
}

/// Pasta de dados da Ayla, a mesma que o Tauri usa (`app_data_dir`).
pub fn data_root() -> PathBuf {
    std::env::var_os("AYLA_DATA_DIR").map(PathBuf::from)
        .unwrap_or_else(|| xdg("XDG_DATA_HOME", ".local/share").join("dev.feit.ayla"))
}

// GApplication encaminha as ativações à instância dona do nome no D-Bus da sessão.
pub fn register(launch: Launch, open: Option<OpenRequest>) -> Result<Option<(gio::Application, mpsc::Receiver<Launch>)>, String> {
    let root = data_root();
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
    let action = gio::SimpleAction::new("open-files", Some(webkit2gtk::glib::VariantTy::STRING));
    let sender = tx.clone();
    action.connect_activate(move |_, parameter| {
        let Some(request) = parameter.and_then(|p| p.str()).and_then(|text| serde_json::from_str::<OpenRequest>(text).ok()) else { return };
        let _ = sender.send(enqueue(request));
    });
    instance.add_action(&action);
    instance.register(gio::Cancellable::NONE).map_err(|e| e.to_string())?;
    if instance.is_remote() {
        if let Some(request) = open {
            let text = serde_json::to_string(&request).map_err(|e| e.to_string())?;
            instance.activate_action("open-files", Some(&text.to_variant()));
            if let Some(bus) = instance.dbus_connection() { bus.flush_sync(gio::Cancellable::NONE).map_err(|e| e.to_string())?; }
        } else if launch != Launch::Login {
            instance.activate_action(if launch == Launch::Quick { "quick" } else { "show" }, None);
            if let Some(bus) = instance.dbus_connection() { bus.flush_sync(gio::Cancellable::NONE).map_err(|e| e.to_string())?; }
        }
        return Ok(None);
    }
    if let Some(request) = open {
        // Primeira instância aberta pela Athena: `Launch::Athena` mostra só a janela dela.
        enqueue(request);
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

/// A pessoa está olhando para a Ayla? No Hyprland, pergunta ao compositor qual janela está ativa
/// (o GTK no Wayland pode continuar "em foco" com a janela escondida). Fora dele, só conta uma
/// janela visível e com foco.
pub fn ayla_focused(app: &AppHandle) -> bool {
    if is_hypr() {
        if let Some(active) = hypr_json("activewindow") {
            return active["pid"].as_u64() == Some(std::process::id() as u64);
        }
    }
    app.webview_windows().values().any(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
}
const NORMAL_SELECTOR: &str = r"initialclass:^(ayla|app|dev\.feit\.ayla)$,initialtitle:^(Ayla)$";

fn panel_title() -> String {
    format!("Ayla · Visão rápida · {}", std::process::id())
}

fn panel_rules(monitor: &Value, radius: u8, sidebar_width: i64, sidebar_gap: u8) -> Option<Vec<String>> {
    let scale = monitor["scale"].as_f64()?;
    if scale <= 0.0 { return None; }
    let (mut width, mut height) = (monitor["width"].as_i64()?, monitor["height"].as_i64()?);
    if monitor["transform"].as_i64().unwrap_or(0) % 2 != 0 {
        std::mem::swap(&mut width, &mut height);
    }
    // A regra estática move usa coordenadas lógicas relativas ao monitor escolhido.
    let inset = |i: usize| monitor["reserved"][i].as_i64().unwrap_or(0);
    let width = ((width as f64 / scale) as i64 - inset(0) - inset(2) - 10).clamp(1, sidebar_width);
    // O painel direito aplica esta margem dentro da área reservada pelo compositor.
    let gap = i64::from(sidebar_gap);
    let height = ((height as f64 / scale) as i64 - inset(1) - inset(3) - 2 * gap).max(1);
    let id = monitor["id"].as_i64()?;
    Some(vec![
        "float".into(), format!("monitor {id}"), format!("size {width} {height}"),
        format!("move {} {}", inset(0) + 5, inset(1) + gap), "bordersize 0".into(),
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
fn quickshell_appearance() -> String {
    fs::read_to_string(xdg("XDG_CONFIG_HOME", ".config").join("quickshell/ii/modules/common/Appearance.qml"))
        .unwrap_or_default()
}
fn sidebar_width(qml: &str) -> i64 {
    qml_value(qml, "property real sidebarWidth:")
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|width| *width > 0)
        .unwrap_or(460)
}
fn sidebar_gap(qml: &str) -> u8 {
    qml_value(qml, "property real hyprlandGapsOut:")
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|gap| (0.0..=f64::from(u8::MAX)).contains(gap))
        .map(|gap| gap.round() as u8)
        .unwrap_or(5)
}
fn system_theme() -> Value {
    let colors = material_palette(&xdg("XDG_STATE_HOME",".local/state").join("quickshell/user/generated/colors.json"));
    let qml = quickshell_appearance();
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
    let radius = corner.map(|r| r.saturating_sub(sidebar_gap(&qml)).saturating_sub(1))
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
            let qml = quickshell_appearance();
            Some(panel_rules(&monitor, radius, sidebar_width(&qml), sidebar_gap(&qml))
                .ok_or("Geometria de monitor inválida")?)
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
    if mode == Launch::Athena {
        return show_athena(app);
    }
    match mode {
        Launch::Login if app.state::<Core>().prefs()?.desktop.background => Ok(()),
        Launch::Quick if quick && (pending || window.is_visible().unwrap_or(false)) => close(app, &window),
        _ => set_mode(app, mode == Launch::Quick),
    }?;
    if mode == Launch::App && !OPEN_QUEUE.lock().unwrap().is_empty() {
        app.emit("open-files", ()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn close(app: &AppHandle, window: &WebviewWindow) -> Result<(), String> {
    click_away(window, false)?;
    let background = app.state::<Core>().prefs()?.desktop.background;
    app.state::<Mutex<View>>().lock().unwrap().cancel_placement();
    window.hide().map_err(|e| e.to_string())?;
    if !background { app.exit(0); }
    Ok(())
}

// O executável instalado pode ter sido substituído enquanto esta sessão rodava.
static RESTART_PATH: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

pub fn restart_requested() -> bool { RESTART_PATH.get().is_some() }

fn restart_path() -> Result<PathBuf, String> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;
    let running = std::env::current_exe().map_err(|e| e.to_string())?;
    let bytes = running.as_os_str().as_bytes();
    let path = Path::new(std::ffi::OsStr::from_bytes(bytes.strip_suffix(b" (deleted)").unwrap_or(bytes)));
    let metadata = path.metadata().map_err(|_| "Não encontrei o executável instalado para reiniciar a Ayla.")?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err("O arquivo instalado da Ayla não é executável.".into());
    }
    Ok(path.to_owned())
}

pub fn relaunch_if_requested() {
    use std::os::unix::process::CommandExt;
    if let Some(path) = RESTART_PATH.get() {
        let error = Command::new(path).exec();
        eprintln!("Não foi possível reabrir a Ayla: {error}");
        std::process::exit(1);
    }
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
        "restart" => { let _ = RESTART_PATH.set(restart_path()?); app.exit(0); Ok(()) }
        _ => Err("Ação de janela desconhecida".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn positions_the_panel_on_the_focused_scaled_rotated_output() {
        let m=json!({"id":3,"x":-900,"y":70,"width":1800,"height":1200,"scale":1.5,"transform":1,"reserved":[18,30,0,12],"activeWorkspace":{"id":4}});
        let qml = "property real sidebarWidth: 480 // painel direito\nproperty real hyprlandGapsOut: 7.0";
        let rules = panel_rules(&m, 17, sidebar_width(qml), sidebar_gap(qml)).unwrap();
        assert!(rules.iter().any(|r| r == "size 480 1144"));
        assert!(rules.iter().any(|r| r == "move 23 37"));
        assert!(rules.iter().any(|r| r == "monitor 3"));
        assert_eq!(sidebar_width(""), 460);
        assert_eq!(sidebar_gap(""), 5);
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

#[cfg(test)]
mod icon_tests {
    use super::*;
    use base64::Engine;

    fn png(side: i32) -> String {
        let pixbuf = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, side, side).unwrap();
        pixbuf.fill(0xff9f43ff);
        let bytes = pixbuf.save_to_bufferv("png", &[]).unwrap();
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn icone_versionado_troca_o_nome_e_a_entrada() {
        let base = std::env::temp_dir().join(format!("ayla-icones-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let icons = base.join("hicolor/512x512/apps");
        fs::create_dir_all(&icons).unwrap();
        fs::write(icons.join("dev.feit.ayla.png"), b"instalador").unwrap();
        fs::write(icons.join("dev.feit.ayla.iris-0badf00d.png"), b"iris").unwrap();
        let (first, changed) = install_icon(&icons, "dev.feit.ayla", b"um").unwrap();
        assert!(changed && first.starts_with("dev.feit.ayla-") && first.len() == "dev.feit.ayla-".len() + 8);
        assert_eq!(install_icon(&icons, "dev.feit.ayla", b"um").unwrap(), (first.clone(), false), "mesmo desenho, nada muda");
        let (second, _) = install_icon(&icons, "dev.feit.ayla", b"dois").unwrap();
        assert_ne!(first, second);
        assert!(!icons.join(format!("{first}.png")).exists(), "versão anterior removida");
        assert!(icons.join("dev.feit.ayla.png").exists() && icons.join("dev.feit.ayla.iris-0badf00d.png").exists(), "outros ícones ficam");

        let entry = base.join("dev.feit.ayla.desktop");
        fs::write(&entry, "[Desktop Entry]\nName=Ayla\nIcon=dev.feit.ayla\nExec=ayla\n").unwrap();
        assert!(set_entry_icon(&entry, &second).unwrap());
        assert_eq!(fs::read_to_string(&entry).unwrap(), format!("[Desktop Entry]\nName=Ayla\nIcon={second}\nExec=ayla\n"));
        assert!(!set_entry_icon(&entry, &second).unwrap());
        assert!(!set_entry_icon(&base.join("ausente.desktop"), &second).unwrap());

        fs::write(base.join("hicolor/icon-theme.cache"), b"velho").unwrap();
        refresh_icon_cache(&icons);
        let cache = fs::read(base.join("hicolor/icon-theme.cache")).ok();
        assert_ne!(cache.as_deref(), Some(&b"velho"[..]), "cache antigo refeito ou removido");
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn abrir_com_le_o_alvo_e_caminhos_absolutos() {
        let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>().into_iter();
        assert_eq!(OpenRequest::parse(args(&["--quick"])), None);
        assert_eq!(OpenRequest::parse(args(&["--abrir", "ayla"])), None, "sem arquivo, abre normal");
        assert_eq!(OpenRequest::parse(args(&["--abrir", "outro", "/tmp/a.md"])), None);
        let request = OpenRequest::parse(args(&["--abrir", "athena", "/tmp/a b.md", "file:///tmp/c%20d.txt", "rel.md"])).unwrap();
        let cwd = std::env::current_dir().unwrap().join("rel.md").to_string_lossy().into_owned();
        assert_eq!(request, OpenRequest { target: "athena".into(), paths: vec!["/tmp/a b.md".into(), "/tmp/c d.txt".into(), cwd] });
    }

    #[test]
    fn icone_da_aparencia_so_aceita_png_512() {
        let (bytes, rgba) = decode_icon(&png(512)).unwrap();
        assert!(bytes.starts_with(b"\x89PNG"));
        assert_eq!(rgba.len(), 512 * 512 * 4);
        assert_eq!(&rgba[..4], &[0xff, 0x9f, 0x43, 0xff]);
        assert!(decode_icon(&png(256)).is_err(), "tamanho errado");
        assert!(decode_icon("não é base64").is_err());
        assert!(decode_icon(&base64::engine::general_purpose::STANDARD.encode(b"GIF89a....")).is_err());
    }
}
