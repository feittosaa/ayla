mod core;
mod desktop;
use core::services::Core;
use std::fs::File;
use std::io::{self, Read};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tauri::Manager;

#[derive(Default)]
struct ManagedBackend(Mutex<Option<BackendState>>);

struct BackendState {
    child: Mutex<Option<Child>>,
    url: String,
    token: String,
}

impl BackendState {
    fn start() -> io::Result<Self> {
        // P0: desenvolvimento Linux. Distribuição será definida na próxima etapa.
        if !cfg!(target_os = "linux") || !cfg!(debug_assertions) {
            return Err(io::Error::other(
                "Este P0 suporta desenvolvimento Linux; instaladores serão preparados depois.",
            ));
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| io::Error::other("Diretório do projeto não encontrado"))?;
        let backend = root.join("backend");
        let python = std::env::var_os("AYLA_PYTHON")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| root.join(".venv/bin/python"));
        // Aleatoriedade do sistema operacional, sem dependência adicional.
        let mut random = [0u8; 32];
        File::open("/dev/urandom")?.read_exact(&mut random)?;
        let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let child = Command::new(python)
            .arg(backend.join("run.py"))
            .current_dir(&backend)
            .env("AYLA_TOKEN", &token)
            .env("AYLA_PORT", port.to_string())
            .stdin(Stdio::null())
            .spawn()?;
        Ok(Self {
            child: Mutex::new(Some(child)),
            url: format!("http://127.0.0.1:{port}"),
            token,
        })
    }

    fn stop(&self) {
        if let Ok(mut guard) = self.child.lock() {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

impl Drop for BackendState {
    fn drop(&mut self) {
        self.stop();
    }
}

#[tauri::command]
fn backend_connection(
    state: tauri::State<'_, ManagedBackend>,
) -> Result<serde_json::Value, String> {
    let mut backend = state.0.lock().map_err(|_| "Falha ao acessar a Ayla")?;
    if backend.is_none() {
        *backend = Some(
            BackendState::start().map_err(|_| "Não foi possível iniciar o serviço de conversa")?,
        );
    }
    let state = backend.as_ref().unwrap();
    let mut guard = state.child.lock().map_err(|_| "Falha ao acessar a Ayla")?;
    let child = guard.as_mut().ok_or("Ayla encerrada")?;
    if child
        .try_wait()
        .map_err(|_| "Falha ao consultar a Ayla")?
        .is_some()
    {
        return Err("O serviço da Ayla não iniciou".into());
    }
    Ok(serde_json::json!({ "url": state.url, "token": state.token }))
}

/// O WebKitGTK nega mídia por padrão. Libera só áudio (microfone), pedido pela
/// própria interface da Ayla após clique; câmera e outras permissões seguem negadas.
#[cfg(target_os = "linux")]
fn allow_microphone(app: &tauri::App) {
    use webkit2gtk::{glib::Cast, PermissionRequestExt, SettingsExt, UserMediaPermissionRequest, UserMediaPermissionRequestExt, WebViewExt};
    let Some(window) = app.get_webview_window("main") else { return };
    let _ = window.with_webview(|webview| {
        let view = webview.inner();
        if let Some(settings) = WebViewExt::settings(&view) {
            settings.set_enable_media_stream(true);
        }
        view.connect_permission_request(|_, request| {
            match request.downcast_ref::<UserMediaPermissionRequest>() {
                Some(media) if media.is_for_audio_device() && !media.is_for_video_device() => request.allow(),
                _ => request.deny(),
            }
            true
        });
    });
}
#[cfg(not(target_os = "linux"))]
fn allow_microphone(_: &tauri::App) {}

/// `ayla --instalar-moneta`: baixa a Moneta junto da Ayla (opção do `install.sh`), sem abrir janela.
fn install_organ_cli() -> Option<i32> {
    if std::env::args().nth(1).as_deref() != Some("--instalar-moneta") {
        return None;
    }
    let organ = core::organs::Organ::moneta(&desktop::data_root());
    Some(match organ.install(&|step| println!("{step}…")) {
        Ok(()) => { println!("Moneta {} pronta. Ela aparece na lateral da Ayla.", organ.manifest.tag); 0 }
        Err(error) => { eprintln!("{error}"); 1 }
    })
}

/// Janelas abertas quando a Ayla foi encerrada por sinal (lido por quem a reinicia).
const OPEN_WINDOWS: &str = "janelas-abertas.json";

/// Anota as janelas abertas (ex.: a Athena separada) e onde estão, para a próxima abertura
/// (reinício pelo botão, instalação ou fim da sessão) reabri-las no lugar; o arquivo some ao abrir.
pub fn remember_windows(app: &tauri::AppHandle) {
    let open: Vec<&str> = ["main", desktop::ATHENA_WINDOW].into_iter()
        .filter(|label| app.get_webview_window(label).is_some_and(|w| w.is_visible().unwrap_or(false))).collect();
    let saved = serde_json::json!({"windows": open, "places": desktop::window_places()});
    if let Some(core) = app.try_state::<Core>() {
        let _ = std::fs::write(core.root.join(OPEN_WINDOWS), saved.to_string());
    }
}

pub fn run() {
    if let Some(code) = install_organ_cli() {
        std::process::exit(code);
    }
    let initial = desktop::Launch::from_args();
    let (instance, requests) = match desktop::register(initial, desktop::OpenRequest::from_args()) {
        Ok(Some(primary)) => primary,
        Ok(None) => return,
        Err(error) => { eprintln!("Falha ao registrar a Ayla na sessão: {error}"); return; }
    };
    let exit_code = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        // Camada de compatibilidade da Moneta: a página dela aparece dentro da Ayla e cada pedido
        // segue para o servidor local da Moneta com a chave de acesso, sem expô-la à página.
        .register_asynchronous_uri_scheme_protocol(core::organs::SCHEME, |context, request, responder| {
            use tauri::http::{Response, StatusCode};
            let label = context.webview_label();
            if label != "main" && label != core::organs::SEPARATE_WINDOW {
                responder.respond(Response::builder().status(StatusCode::FORBIDDEN).body(Vec::new()).unwrap_or_default());
                return;
            }
            let app = context.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                let organ = app.state::<core::organs::Organ>();
                responder.respond(organ.forward(request).await);
            });
        })
        // Anexos das notas da Athena (imagens e PDFs), lidos da pasta de cada raiz.
        .register_asynchronous_uri_scheme_protocol("athena-file", |context, request, responder| {
            use tauri::http::{Response, StatusCode};
            if context.webview_label() != "main" {
                responder.respond(Response::builder().status(StatusCode::FORBIDDEN).body(Vec::new()).unwrap_or_default());
                return;
            }
            let app = context.app_handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                let response = match app.try_state::<Core>() {
                    Some(core) => core::services::athena_file(&core, &request),
                    None => Response::builder().status(StatusCode::SERVICE_UNAVAILABLE).body(Vec::new()).unwrap_or_default(),
                };
                responder.respond(response);
            });
        })
        // Biblioteca da Athena: livros (EPUB) e mangás (CBZ) das fontes, por trechos.
        .register_asynchronous_uri_scheme_protocol("biblioteca-file", |context, request, responder| {
            use tauri::http::{Response, StatusCode};
            if context.webview_label() != "main" {
                responder.respond(Response::builder().status(StatusCode::FORBIDDEN).body(Vec::new()).unwrap_or_default());
                return;
            }
            let app = context.app_handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                let response = match app.try_state::<Core>() {
                    Some(core) => core::services::library_file(&core, &request),
                    None => Response::builder().status(StatusCode::SERVICE_UNAVAILABLE).body(Vec::new()).unwrap_or_default(),
                };
                responder.respond(response);
            });
        })
        // Iris: imagens, vídeos e PDFs guardados na Ayla (anexos do chat e imagens geradas).
        .register_asynchronous_uri_scheme_protocol("iris-file", |context, request, responder| {
            use tauri::http::{Response, StatusCode};
            if context.webview_label() != "main" {
                responder.respond(Response::builder().status(StatusCode::FORBIDDEN).body(Vec::new()).unwrap_or_default());
                return;
            }
            let app = context.app_handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                let response = match app.try_state::<Core>() {
                    Some(core) => core::services::iris_file(&core, &request),
                    None => Response::builder().status(StatusCode::SERVICE_UNAVAILABLE).body(Vec::new()).unwrap_or_default(),
                };
                responder.respond(response);
            });
        })
        .setup(move |app| {
            let directory = std::env::var_os("AYLA_DATA_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or(app.path().app_data_dir()?);
            let core = Core::open(directory.clone()).map_err(io::Error::other)?;
            // Reinício: as janelas voltam ao workspace e ao monitor em que estavam (Hyprland), e a
            // Athena separada, se estava aberta, abre de novo (exceto no login em segundo plano).
            let mut reopen_athena = false;
            if let Some(saved) = std::fs::read_to_string(directory.join(OPEN_WINDOWS)).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
                desktop::restore_places(saved["places"].clone());
                reopen_athena = saved["windows"].as_array().is_some_and(|w| w.iter().any(|l| l == desktop::ATHENA_WINDOW));
            }
            let _ = std::fs::remove_file(directory.join(OPEN_WINDOWS));
            // Workers de uma Ayla que caiu sem encerrar continuariam ocupando RAM e VRAM.
            let reaped = core.runtime.reap_orphans();
            if reaped > 0 { eprintln!("Ayla: {reaped} modelo(s) local(is) de uma execução anterior encerrado(s)."); }
            // SIGTERM (pkill, fim da sessão), SIGHUP e SIGINT saem pelo caminho normal, que encerra
            // os modelos locais e o contêiner da Moneta.
            #[cfg(unix)]
            {
                let exit = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    use tokio::signal::unix::{signal, SignalKind};
                    let (Ok(mut term), Ok(mut hup), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::hangup()), signal(SignalKind::interrupt())) else { return };
                    tokio::select! { _ = term.recv() => {}, _ = hup.recv() => {}, _ = int.recv() => {} }
                    remember_windows(&exit);
                    exit.exit(0);
                });
            }
            let weak = std::sync::Arc::downgrade(&core.runtime);
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    match weak.upgrade() {
                        Some(runtime) => runtime.idle_check(),
                        None => break,
                    }
                }
            });
            allow_microphone(app);
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    core::services::memory_tick(&handle).await;
                }
            });
            // Athena: Git das notas (puxar ao abrir e intervalo do obsidian-git), conferido a cada minuto.
            let notes = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                let mut boot = true;
                loop {
                    core::services::athena_git_tick(&notes, boot).await;
                    boot = false;
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                }
            });
            // Lembretes: confere ao abrir (os perdidos avisam como atrasados) e a cada 2 s
            // (consulta barata ao índice por horário), para avisos de segundos chegarem na hora.
            let reminders = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    core::services::reminder_tick(&reminders).await;
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            });
            // Rotinas (Hand › Rotinas): conferidas a cada 20 s; a primeira volta, logo depois de
            // entrar na sessão, dispara as de "ao ligar o PC".
            let routines = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(8)).await;
                loop {
                    core::services::rotinas::rotinas_tick(&routines).await;
                    tokio::time::sleep(std::time::Duration::from_secs(20)).await;
                }
            });
            app.manage(core::organs::Organ::moneta(&directory));
            app.manage(core);
            app.manage(core::pad::Pad::default());
            app.manage(ManagedBackend::default());
            app.manage(Mutex::new(desktop::View::default()));
            desktop::launch(app.handle(), initial).map_err(io::Error::other)?;
            // No login (segundo plano) nenhuma janela aparece, nem a Athena anotada no fim da sessão.
            if reopen_athena && !matches!(initial, desktop::Launch::Athena | desktop::Launch::Login) {
                let _ = desktop::show_athena(app.handle());
            }
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                for request in requests {
                    let app = handle.clone();
                    let _ = handle.run_on_main_thread(move || {
                        if let Err(error) = desktop::launch(&app, request) {
                            use tauri::Emitter;
                            let _ = app.emit("desktop-warning", error);
                        }
                    });
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            desktop::desktop_view,
            desktop::quick_ready,
            desktop::desktop_action,
            desktop::open_requests,
            core::services::athena_open_external,
            core::services::bases_list,
            core::services::bases_install,
            core::services::bases_remove,
            core::services::bases_validate,
            core::services::bases_approve,
            core::services::bases_discard,
            core::services::image_keys_status,
            core::services::image_key_set,
            core::services::image_save,
            core::services::iris_open_external,
            backend_connection,
            core::services::desktop_snapshot,
            core::services::save_preferences,
            core::services::cloud_key,
            core::services::import_model,
            core::services::remove_model,
            core::services::install_runtime,
            core::services::install_catalog_model,
            core::services::setup_start,
            core::services::transcribe_preview,
            core::services::rename_conversation,
            core::services::delete_conversation,
            core::services::projects,
            core::services::pick_files,
            core::services::cloud_providers,
            core::services::cloud_connect,
            core::services::cloud_login,
            core::services::cloud_disconnect,
            core::services::cloud_models,
            core::services::open_link,
            core::services::open_web,
            core::services::generate_image,
            core::services::file_data,
            core::services::export_file,
            core::services::add_file,
            core::services::add_file_data,
            core::services::capture_screen,
            core::services::remove_file,
            core::services::project_sources::project_links,
            core::services::project_sources::project_source_options,
            core::services::project_sources::project_link,
            core::services::project_sources::project_unlink,
            core::services::project_sources::project_import_database,
            core::services::project_files,
            core::services::create_project,
            core::services::update_project,
            core::services::set_project_look,
            core::services::delete_project,
            core::services::memory_list,
            core::services::memory_save,
            core::services::memory_delete,
            core::services::conversation_memories,
            core::services::cancel_job,
            core::services::unload_models,
            core::services::conversations,
            core::services::create_conversation,
            core::services::conversation_messages,
            core::services::conversation_select,
            core::services::conversation_consent,
            core::services::chat,
            core::services::cloud_references,
            core::services::reminders_list,
            core::notify::notify_test,
            core::services::athena_pick_folder,
            core::services::athena_tree,
            core::services::athena_add_vault,
            core::services::athena_remove_vault,
            core::services::athena_mkdir,
            core::services::athena_backlinks,
            core::services::athena_rename,
            core::services::athena_rename_folder,
            core::services::athena_overview,
            core::services::athena_attachment,
            core::services::athena_read,
            core::services::athena_write,
            core::services::athena_paste_image,
            core::services::athena_set_banner,
            core::services::athena_import,
            core::services::library_list,
            core::services::library_add_folder,
            core::services::library_remove_folder,
            core::services::library_import,
            core::services::library_progress,
            core::services::library_mode,
            core::services::library_mark,
            core::services::library_trash,
            core::services::library_mkdir,
            core::services::library_move,
            core::services::athena_protect,
            core::services::athena_unlock,
            core::services::athena_lock,
            core::services::athena_unprotect,
            core::services::athena_change_password,
            core::services::athena_recover,
            core::services::athena_recovery_status,
            core::services::athena_recovery_create,
            core::services::athena_recovery_activate,
            core::services::athena_create,
            core::services::athena_daily,
            core::services::athena_trash,
            core::services::athena_search,
            core::services::athena_open_folder,
            core::services::athena_trash_folder,
            core::services::media_open,
            desktop::appearance_icon,
            core::services::save_appearance,
            core::services::athena_move_file,
            core::services::athena_rename_file,
            core::services::athena_move_root,
            core::services::iris_library,
            core::services::iris_trash_image,
            core::services::iris_rename,
            core::services::iris_change_folder,
            core::services::iris_move_folder,
            core::services::athena_open_obsidian,
            core::services::athena_git_status,
            core::services::athena_git_sync,
            core::services::reminder_delete,
            core::services::reminder_snooze,
            core::services::backup_data,
            core::services::semantic_memory,
            core::services::voice_task,
            core::services::import_legacy,
            core::organs::organ_status,
            core::organs::organ_install,
            core::organs::organ_start,
            core::organs::organ_theme,
            core::organs::organ_remove,
            core::organs::organ_open_separate,
            core::organs::organ_local_words,
            core::research::suggests_web_search,
            core::research::automatic_check,
            core::research::cloud_notes_check,
            core::services::set_response_source,
            core::services::source_remove,
            core::services::iris_upscale,
            core::services::iris_upscaler,
            core::services::iris_upscale_batch,
            core::services::iris_trash_batch,
            core::services::iris_move_batch,
            core::services::iris_destinations,
            core::services::ficha_status,
            core::services::ficha_pagina,
            core::services::ficha_modelo,
            core::services::ficha_modelo_salvar,
            core::services::ficha_abrir,
            core::services::ficha_previa,
            core::services::ficha_aplicar,
            core::services::ficha_cancelar,
            core::services::hand::hand_estado,
            core::services::hand::hand_pastas,
            core::services::hand::hand_capturar,
            core::services::hand::hand_buscar,
            core::services::hand::hand_planejar,
            core::services::hand::hand_aplicar,
            core::services::hand::hand_historico,
            core::services::hand::hand_desfazer,
            core::services::hand::hand_programas,
            core::services::hand::hand_abrir_programa,
            core::services::hand::hand_abrir,
            core::services::hand::hand_comando,
            core::services::hand::hand_janelas,
            core::services::hand::hand_focar,
            core::services::hand::hand_fechar,
            core::services::hand::hand_digitar,
            core::services::hand::hand_teclas,
            core::services::hand::hand_clicar,
            core::services::hand::hand_rolar,
            core::services::hand::hand_cursor,
            core::services::hand::hand_tarefa_planejar,
            core::services::hand::hand_tarefa_executar,
            core::services::hand::hand_tarefa_responder,
            core::services::hand::hand_tarefa_parar,
            core::services::hand::hand_papel_de_parede,
            core::services::hand::hand_miniatura,
            core::services::hand_sistema::hand_programas_grade,
            core::services::hand_sistema::hand_programas_avaliar,
            core::services::hand_sistema::hand_icone,
            core::services::hand_sistema::hand_programa_previa,
            core::services::hand_sistema::hand_programa_desinstalar,
            core::services::hand_sistema::hand_atalhos,
            core::services::hand_sistema::hand_atalho_mudar,
            core::services::hand_sistema::hand_macros,
            core::services::hand_sistema::hand_macro_salvar,
            core::services::hand_sistema::hand_macro_apagar,
            core::services::hand_sistema::hand_macro_rodar,
            core::services::hand_sistema::hand_gravar_comecar,
            core::services::hand_sistema::hand_gravar_parar,
            core::services::hand_sistema::hand_dotfiles_estado,
            core::services::hand_sistema::hand_dotfiles_conteudo,
            core::services::hand_sistema::hand_dotfiles_explicar,
            core::services::hand_sistema::hand_dotfiles_sincronizar,
            core::services::rotinas::rotinas_lista,
            core::services::design_doc::design_documento,
            core::services::feriados::feriados_estado,
            core::services::feriados::feriados_buscar,
            core::services::design_doc::design_documento_base,
            core::services::design_doc::design_documento_atual,
            core::services::design_doc::design_documento_gravar,
            core::services::rotinas::rotina_salvar,
            core::services::rotinas::rotina_ativar,
            core::services::rotinas::rotina_apagar,
            core::services::rotinas::rotina_testar,
            core::services::backups_abrir,
            core::services::iris_editor,
            core::services::conversation_discard_empty,
            core::services::iris_edit_image,
            core::services::iris_edit_video,
            core::services::design_list,
            core::services::design_read,
            core::services::design_save,
            core::services::design_delete,
            core::services::design_export,
            core::services::design_fonts,
            core::services::design_font,
            core::services::design_icons,
            core::services::design_image, core::services::design_image_sources, core::services::design_image_search, core::services::design_image_fetch,
            core::services::calendar_range,
            core::services::calendar_save,
            core::pad::pad_open_dialog,
            core::pad::pad_save_dialog,
            core::pad::pad_read,
            core::pad::pad_write,
            core::pad::pad_modified,
            core::pad::pad_requests,
            core::pad::pad_session_load,
            core::pad::pad_session_save,
            core::services::source_restore,
            core::version::app_version
        ])
        .build(tauri::generate_context!())
        .expect("Não foi possível iniciar a Ayla")
        .run_return(|app, event| {
            if let tauri::RunEvent::WindowEvent { ref label, event: tauri::WindowEvent::CloseRequested { ref api, .. }, .. } = event {
                if let Some(win) = app.get_webview_window(label) {
                    if app.state::<Core>().prefs().is_ok_and(|p| p.desktop.background) && win.hide().is_ok() {
                        api.prevent_close();
                        app.state::<Mutex<desktop::View>>().lock().map(|mut v| v.cancel_placement()).ok();
                    }
                }
            }
            // Athena aberta sozinha: fechar a janela dela encerra a Ayla se a conversa não estiver
            // aberta e o segundo plano estiver desligado (senão o processo ficaria invisível).
            if let tauri::RunEvent::WindowEvent { ref label, event: tauri::WindowEvent::Destroyed, .. } = event {
                if label == desktop::ATHENA_WINDOW {
                    let main_hidden = app.get_webview_window("main").is_none_or(|w| !w.is_visible().unwrap_or(false));
                    let background = app.state::<Core>().prefs().is_ok_and(|p| p.desktop.background);
                    if main_hidden && !background { app.exit(0); }
                }
            }
            if let tauri::RunEvent::Exit = event {
                desktop::clear_rules();
                if let Some(core) = app.try_state::<Core>() {
                    core::services::hand::hand_encerrar(&core);
                    core.shutdown();
                }
                // O contêiner da Moneta acompanha a Ayla; a versão separada continua como está.
                if let Some(organ) = app.try_state::<core::organs::Organ>() {
                    // No reinício, espera a Moneta gravar e sair antes de a nova Ayla subi-la de novo.
                    organ.stop(desktop::restart_requested());
                }
                if let Some(state) = app.try_state::<ManagedBackend>() {
                    if let Ok(mut backend) = state.0.lock() {
                        if let Some(backend) = backend.take() {
                            backend.stop();
                        }
                    }
                }
            }
        });
    drop(instance);
    desktop::relaunch_if_requested();
    std::process::exit(exit_code);
}

// O backend Python legado só sobe em build de desenvolvimento (ver `BackendState::start`): em
// `--release` ele recusa de propósito, então estes testes só valem sem otimização.
#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpStream;
    use std::time::{Duration, Instant};

    fn health(state: &BackendState, authenticated: bool) -> io::Result<String> {
        let address = state.url.trim_start_matches("http://");
        let mut socket = TcpStream::connect(address)?;
        socket.set_read_timeout(Some(Duration::from_secs(2)))?;
        let header = if authenticated {
            format!("X-Ayla-Token: {}\r\n", state.token)
        } else {
            String::new()
        };
        write!(
            socket,
            "GET /health HTTP/1.1\r\nHost: localhost\r\n{header}Connection: close\r\n\r\n"
        )?;
        let mut response = String::new();
        socket.read_to_string(&mut response)?;
        Ok(response)
    }

    #[test]
    fn backend_lifecycle_without_ai() {
        // A execução deste teste deve usar banco temporário e IA desligada.
        let database =
            std::env::var("AYLA_DB_PATH").expect("Defina AYLA_DB_PATH temporário para o teste");
        assert!(
            !Path::new(&database).exists(),
            "O teste exige um banco novo e descartável"
        );
        assert_eq!(std::env::var("AYLA_LOCAL_ENABLED").as_deref(), Ok("false"));
        assert_eq!(std::env::var("AYLA_CLOUD_ENABLED").as_deref(), Ok("false"));
        let start = Instant::now();
        let state = BackendState::start().expect("Iniciar backend Python");
        let mut ready = false;
        while start.elapsed() < Duration::from_secs(10) {
            if let Ok(response) = health(&state, true) {
                if response.starts_with("HTTP/1.1 200") && response.contains("\"status\":\"ok\"") {
                    assert!(response.contains("\"ai_configured\":false"));
                    ready = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            ready,
            "Backend precisa ficar pronto sem modelos ou rede externa"
        );
        println!(
            "Backend pronto sem IA em {} ms",
            start.elapsed().as_millis()
        );
        let unauthorized = health(&state, false).expect("Consultar sem token");
        assert!(unauthorized.starts_with("HTTP/1.1 401"));
        assert!(!unauthorized.contains(&state.token));
        assert!(
            !Path::new(&database).exists(),
            "Inicialização e health não devem criar banco"
        );
        state.stop();
        assert!(state.child.lock().unwrap().is_none());
        assert!(TcpStream::connect(state.url.trim_start_matches("http://")).is_err());
    }
}
