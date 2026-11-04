#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::{Context, Result};
use octo_core::Config;
use octo_desktop::{Server, init_script, start_server};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

fn web_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    let bundled = app.path().resource_dir().ok().map(|d| d.join("web"));
    bundled
        .into_iter()
        .chain([PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../web/dist"
        ))])
        .find(|p| p.join("index.html").is_file())
}

fn setup(app: &mut tauri::App) -> Result<()> {
    let handle = app.handle().clone();
    let web = web_dir(&handle);
    let server = tauri::async_runtime::block_on(start_server(Config::new(None), web))?;
    let url = server.url().parse().context("server url")?;
    WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
        .title("Octo")
        .inner_size(1200.0, 800.0)
        .min_inner_size(640.0, 480.0)
        .initialization_script(init_script(&server.token))
        .build()?;
    app.manage(Mutex::new(Some(server)));
    Ok(())
}

fn main() {
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_window_state::Builder::default().build());
    // The updater needs the owner's public key at build time; without it the plugin is skipped.
    if let Some(pubkey) = option_env!("OCTO_UPDATER_PUBKEY").filter(|k| !k.is_empty()) {
        builder = builder.plugin(tauri_plugin_updater::Builder::new().pubkey(pubkey).build());
    }
    let app = builder
        .setup(|app| setup(app).map_err(Into::into))
        .build(tauri::generate_context!());
    let app = match app {
        Ok(app) => app,
        Err(e) => {
            eprintln!("octo: {e}");
            std::process::exit(1);
        }
    };
    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            let server = handle
                .try_state::<Mutex<Option<Server>>>()
                .and_then(|s| s.lock().ok().and_then(|mut g| g.take()));
            if let Some(server) = server {
                tauri::async_runtime::block_on(server.shutdown());
            }
        }
    });
}
