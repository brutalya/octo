use crate::{Server, init_script, is_app_url, start_server};
use anyhow::{Context, Result};
use octo_core::Config;
use octo_server::WebAssets;
use std::sync::Arc;
use std::sync::Mutex;
use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_opener::OpenerExt;

/// The UI embedded by Tauri (`frontendDist`); same lookup on every platform.
fn embedded_web(app: &tauri::AppHandle) -> WebAssets {
    let handle = app.clone();
    WebAssets::Embedded(Arc::new(move |path| {
        handle
            .asset_resolver()
            .get(path.to_string())
            .map(|a| (a.bytes, a.mime_type))
    }))
}

/// Navigation guard: pages of the embedded server load in the webview, anything else (a link
/// that is not `target=_blank`, a redirect) is refused and, if it is a web link, opened in the
/// system browser.
fn open_outside(handle: &tauri::AppHandle, url: &tauri::Url, addr: std::net::SocketAddr) -> bool {
    if is_app_url(url, addr) {
        return true;
    }
    if matches!(url.scheme(), "http" | "https") {
        let _ = handle.opener().open_url(url.as_str(), None::<&str>);
    }
    false
}

fn setup(app: &mut tauri::App) -> Result<()> {
    let handle = app.handle().clone();
    let web = Some(embedded_web(&handle));
    let data_dir = app.path().app_data_dir().context("app data dir")?;
    std::fs::create_dir_all(&data_dir).context("create app data dir")?;
    let server = tauri::async_runtime::block_on(start_server(Config::new(Some(data_dir)), web))?;
    let url: tauri::Url = server.url().parse().context("server url")?;
    let addr = server.addr;
    let nav_handle = handle.clone();
    let new_handle = handle.clone();
    let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
        .title("Octo")
        .on_navigation(move |u| open_outside(&nav_handle, u, addr))
        .on_new_window(move |u, _| {
            // `target=_blank` links: never a second webview, always the system browser.
            open_outside(&new_handle, &u, addr);
            tauri::webview::NewWindowResponse::Deny
        });
    #[cfg(desktop)]
    let window = window
        .inner_size(1200.0, 800.0)
        .min_inner_size(640.0, 480.0);
    window
        .initialization_script(init_script(
            &server.token,
            &format!("http://{}", server.addr),
        ))
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
    let builder =
        with_desktop_plugins(tauri::Builder::default().plugin(tauri_plugin_opener::init()));
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
