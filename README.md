# Octo

AI interaction hub: Claude Code / agy CLI, Ollama, OpenRouter. Private / Public spaces.

## Download

[Latest release](../../releases/latest):

| Platform | File |
|---|---|
| Windows | `Octo_<version>_x64-setup.exe` |
| Linux | `.AppImage`, `.deb` |
| Android (arm64) | `octo-<version>-arm64-debug.apk` |
| Server / relay | Docker images `ghcr.io/<owner>/<repo>-node`, `-relay` ([deploy](docs/deploy.md)) |

Android: copy the APK to the phone, allow installing from unknown sources for your file manager/browser, open it. It is a debug build signed with one fixed key, so a new APK installs over the old one and keeps its data (APKs before 0.2.8 had a random key each: uninstall those once).

## Prerequisites (from source)

- Rust (rustup): https://rustup.rs
- Node 22+
- Optional: Claude Code (`claude`), `agy` CLI, Ollama

## Run (dev)

Linux / macOS:

    scripts/dev.sh

Windows (PowerShell):

    scripts\dev.ps1

Starts the server (127.0.0.1:7878) and Vite (http://localhost:5173).

## Production-like (headless server with web UI)

    cd web && npm ci && npm run build
    cd .. && cargo run --release -p octo-server

Open http://127.0.0.1:7878. `web/dist` is built locally and is not in git. Flags: [docs/deploy.md](docs/deploy.md#flags).

## Checks

    cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
    cd web && npm run lint && npm test && npm run build

Some tests use shell fake CLIs and run on unix only.

## Workflow

- `main` is production. Every change (feature, fix, issue) gets its own branch from `main` (`feat/...`, `fix/...`, `docs/...`, `ci/...`), is checked there and merged through a pull request. No direct pushes to `main`.
- CI (`ci.yml`) runs only what a change touches: Rust jobs (Linux: fmt, clippy, tests; Windows: tests) for `crates/`, `Cargo.*`, `.cargo/`; web jobs for `web/`. Docs-only changes run no tests. `ci-ok` is the single status to require.
- Docker images (`docker.yml`) are rebuilt on merges to `main` that change code or deploy files.

## Releases

Bump the version in `crates/octo-desktop/tauri.conf.json` and `crates/octo-desktop/Cargo.toml` in a pull request. When it is merged, `release.yml` tags `v<version>` and publishes a release with the Windows, Linux and Android builds. Merges without a version change publish nothing.

Updater signing key (one time, optional; without it releases build but have no auto-update):

```
cargo tauri signer generate -w ~/.tauri/octo.key
```

Add the private key as repo secret `TAURI_SIGNING_PRIVATE_KEY` (and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` if set), and the public key as repo variable `OCTO_UPDATER_PUBKEY`.

## Desktop (build locally)

Tauri v2 app (`crates/octo-desktop`): embeds the server on a random loopback port with a per-launch token; no sidecar.

Linux also needs `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev build-essential pkg-config`:

```
cargo install tauri-cli --version "^2" --locked
(cd web && npm ci && npm run build)   # the app embeds web/dist; rebuild after web changes
cd crates/octo-desktop
cargo tauri dev      # opens the window
cargo tauri build    # installer in target/release/bundle
```

## Android (build)

Built only in CI; no Android SDK is needed locally. The `gen/android` project is not committed, CI generates it with `cargo tauri android init` and then replaces its `MainActivity.kt` with `crates/octo-desktop/android/MainActivity.kt` (pads the WebView by the system bars and keyboard; Android 15+ draws apps edge to edge). `android.yml` can also be run by hand (Actions → Android → Run workflow); the APK is then a run artifact. Signing: the repo secret `ANDROID_DEBUG_KEYSTORE_BASE64` (base64 of a PKCS12 keystore, alias `androiddebugkey`, store and key password `android`) is passed to Gradle as the injected signing config (`gen/android/gradle.properties`), and CI checks the APK is signed with it; releases fail without it. Keep a copy of the keystore offline: losing or changing it means one uninstall, which wipes the app's data on the phone.

## Deploy

Docker relay/node behind Nginx Proxy Manager, systemd alternative: [docs/deploy.md](docs/deploy.md).

Quick start (relay):

    docker login ghcr.io
    cp deploy/relay/docker-compose.yml . && $EDITOR docker-compose.yml   # set image and domain; AUTO_UPDATE on or off
    docker compose up -d
    # NPM: host relay.example.com -> octo-relay:7443, Websockets ON, SSL
    curl https://relay.example.com/healthz
    docker compose logs octo-relay | grep "relay code"
    # paste the code into Octo: Settings, Network, Create a network (first device)
    # other devices: Add device on the first one, Join a network on the new one

## Security

- The relay sees: node IPs, `network_id` (a hash, not a key), node ids, frame sizes and timing. It does not see: content (Noise IK end to end), keys, names, request streams.
- The relay holds no keys; a malicious relay can only cut the connection. Put it behind a TLS proxy (`wss://`) to protect metadata.
- The deployed relay is private: only nodes with its secret (`?key=` in the relay URL) can connect, so strangers cannot use it to link their own devices. The secret is not an Octo network key; pairing still needs a one-time code and SAS confirmation.
- Two keys: `network_key` (membership and routing, held by every node) and `private_key` (private space sync, 32 bytes, with an epoch).
- Device trust levels: `full` (gets `private_key`, may read private threads and `/files/private`) and `public` (public data only). A new device is `public` by default; the host chooses when confirming the SAS.
- Policy is enforced on the target node before the request is handled; the thread list is filtered for `public`, private threads, files and attachments return 403.
- Files: Private files sync automatically between `full` devices. Public files stay on the device that has them; the Files panel lists every online device's Public files in one tree. With Public auto-sync on (`PATCH /api/settings {public_sync: true}`, off by default, per device) a device copies Public files it lacks from the online devices every minute. It never overwrites or deletes; Delete and Rename in the panel act on every device that has the file.
- Downgrading or removing a device rotates `private_key` (new epoch) and sends it only to the remaining `full` devices. Data copied earlier is not revoked.
