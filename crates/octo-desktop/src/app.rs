use crate::{Server, init_script, start_server};
use anyhow::{Context, Result};
use octo_core::Config;
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
    let data_dir = app.path().app_data_dir().context("app data dir")?;
    std::fs::create_dir_all(&data_dir).context("create app data dir")?;
    let server = tauri::async_runtime::block_on(start_server(Config::new(Some(data_dir)), web))?;
    let url = server.url().parse().context("server url")?;
    let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url)).title("Octo");
    #[cfg(desktop)]
    let window = window
        .inner_size(1200.0, 800.0)
        .min_inner_size(640.0, 480.0);
    window
        .initialization_script(init_script(&server.token))
        .build()?;
    app.manage(Mutex::new(Some(server)));
    Ok(())
}

#[cfg(desktop)]
fn with_desktop_plugins(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    let mut builder = builder
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
    builder
}

#[cfg(mobile)]
fn with_desktop_plugins(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder
}

/// Entry point shared by the desktop binary and the mobile shared library.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = with_desktop_plugins(tauri::Builder::default());
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
