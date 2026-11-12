# Stage 5 — Android (thin client first)

Goal: the phone is a full Octo node (own identity, pairing, relay link, private files with sync) that controls agents on PC/VPS. No CLI providers on the phone.

## Shape
- Same Tauri v2 app (`crates/octo-desktop`) built for Android (`cargo tauri android`). Core + axum on `127.0.0.1:0` with per-launch token, exactly as desktop.
- Providers on Android: none of the CLIs (detection already returns not-installed; `which` finds nothing). Ollama/OpenRouter stay available if configured. Nothing special-cased by `cfg(target_os)` except what the build needs.
- Data dir: Tauri app data dir (`app.path().app_data_dir()`), passed explicitly to core (no `directories` lookup on Android).
- Single-instance and updater plugins: desktop only (`#[cfg(desktop)]`).
- Lifecycle: app in background may be killed — the node reconnects on resume; nothing else.
- Remote is the main mode: when members exist, the node switcher defaults to the last used node (remembered in localStorage).

## Web: phone layout (< 768 px)
- One column. Sidebar (threads, node switcher, settings) becomes a drawer opened by a menu button; files panel becomes a full-screen sheet.
- Composer sticks to the bottom, respects safe-area insets and the on-screen keyboard (`visualViewport`).
- Touch targets ≥ 40 px. No hover-only actions (context actions via long-press or a visible `…` button).
- File upload from the phone: `<input type=file>` (camera/voice recorder via system picker), same Files API.
- Tests first (vitest): drawer opens/closes, thread select closes the drawer, files sheet, layout switch by width (mock `matchMedia`).

## Build
- Built only in CI (no Android SDK on dev machines needed): `android.yml` on tag `v*` and manual dispatch: setup-java 17, Android SDK/NDK via `android-actions/setup-android`, `rustup target add aarch64-linux-android`, `cargo tauri android build --apk --target aarch64`. Unsigned debug-signed APK as artifact; release signing via repo secrets later (documented placeholders).
- README: how to install the APK (download artifact, allow unknown sources).

## Deviations
- `gen/android` is not committed (the whole `crates/octo-desktop/gen/` is gitignored); `android.yml` runs `cargo tauri android init --ci` before building.
- The APK is built with `--debug` (debug-signed, no keystore); the debug manifest also permits cleartext HTTP to the loopback server. Release signing is a later step.
- Entry point lives in the lib (`octo_desktop::run`, crate types `staticlib`/`cdylib`/`rlib`); `main.rs` only calls it. Desktop-only plugins (single-instance, updater, window-state) are target-gated dependencies, and capabilities are split into `default.json` (desktop) and `mobile.json`.
- The UI is served from Tauri's embedded assets (`frontendDist`) on all desktop/mobile builds via `asset_resolver()`, through `octo_server::WebAssets::Embedded`; the `web` resource bundling was removed. The server binary keeps `WebAssets::Dir`.
- Cleartext: the webview loads `http://127.0.0.1:<port>`; Android release builds block cleartext by default, so the APK stays a `--debug` build until a network-security config allowing loopback is added.
- The desktop/phone app starts the node service (`Node::start`) and builds its router with the node attached, exactly like the server binary, so pairing, sync and the tunnel work there. The webview navigates only to `http://127.0.0.1:<port>` of the embedded server (`on_navigation`); other web links (including `target=_blank`, `on_new_window`) open in the system browser via `tauri-plugin-opener`. The injected token script sets `window.__OCTO_TOKEN__` only when `location.origin` equals the server origin.
- `android.yml` runs on manual dispatch only (not on tags), to save CI minutes while Android is not the focus.
