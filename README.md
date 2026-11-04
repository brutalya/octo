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
