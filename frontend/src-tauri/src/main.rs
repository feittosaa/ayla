// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::Command;
use tauri::{AppHandle, Manager};
use rand::{distributions::Alphanumeric, Rng};

fn generate_token() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect()
}

#[cfg(debug_assertions)]
fn start_backend_dev(token: &str) {
    Command::new("python")
        .args(["run.py"])
        .env("AYLA_TOKEN", token)
        .spawn()
        .expect("failed to start backend (dev)");
}

#[cfg(not(debug_assertions))]
fn start_backend_prod(app: &AppHandle, token: &str) {
    let backend_path = app
        .path_resolver()
        .resolve_resource("bin/ayla-backend.exe")
        .expect("backend not found");

    Command::new(backend_path)
        .env("AYLA_TOKEN", token)
        .spawn()
        .expect("failed to start backend");
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let token = generate_token();

            #[cfg(debug_assertions)]
            start_backend_dev(&token);

            #[cfg(not(debug_assertions))]
            start_backend_prod(app, &token);

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error running tauri app");
}
