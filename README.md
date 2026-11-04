# Octo

AI interaction hub: Claude Code / agy CLI, Ollama, OpenRouter. Private / Public spaces.

## Требования / Prerequisites

- Rust (rustup): https://rustup.rs
- Node 20+
- Optional / по желанию: Claude Code (`claude`), `agy` CLI, Ollama

## Запуск / Run (dev)

Linux / macOS:

    scripts/dev.sh

Windows (PowerShell):

    scripts\dev.ps1

Starts the server (127.0.0.1:7878) and Vite (http://localhost:5173).

## Сборка / Production-like

    cd web && npm ci && npm run build
    cd .. && cargo run --release -p octo-server

Open http://127.0.0.1:7878. `web/dist` is built locally and is not in git.

## Проверки / Checks

    cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
    cd web && npm run lint && npm test && npm run build

Some tests use shell fake CLIs and run on unix only.

## Desktop

Tauri v2 app (`crates/octo-desktop`): embeds the server on a random loopback port with a per-launch token; no sidecar.

Build locally (Linux also needs `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev build-essential pkg-config`):

```
cargo install tauri-cli --version "^2" --locked
cd crates/octo-desktop
cargo tauri dev      # builds web/, opens the window
cargo tauri build    # installer in target/release/bundle
```

Release: push a tag `v*` (`git tag v0.1.0 && git push --tags`). `release.yml` builds Windows (NSIS), macOS (dmg), Linux (AppImage, deb) and attaches them to a draft GitHub Release.

Updater signing key (one time, optional; without it releases build but have no auto-update):

```
cargo tauri signer generate -w ~/.tauri/octo.key
```

Add the private key as repo secret `TAURI_SIGNING_PRIVATE_KEY` (and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` if set), and the public key as repo variable `OCTO_UPDATER_PUBKEY`.

## VPS

Файлы в `deploy/` / files in `deploy/`. Build: `cargo build --release -p octo-server`.

- `octo-node.service`: узел, API только на `127.0.0.1:7878`.
- `octo-relay.service`: только relay (`--relay-only --relay-addr`), отдельный порт; отдаёт лишь `/relay` и `/healthz`, API там нет.
- `Caddyfile.example`: `wss://домен/relay` -> `127.0.0.1:7443`. Для relay используйте `relay_url = wss://домен/relay`.
- Сервер не стартует с `--addr` не на loopback без `--token`.
