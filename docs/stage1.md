# Stage 1 — local models, OpenRouter, spaces, files

## Providers
Add HTTP providers next to CLI ones. `ProviderKind { Claude, Antigravity, Ollama, OpenRouter }`.
- **Ollama** (`is_local = true`): native API `POST {base}/api/chat` stream NDJSON; models from `GET /api/tags`. Base default `http://127.0.0.1:11434`. Detect = tags reachable within 1s.
- **OpenRouter** (`is_local = false`): `POST https://openrouter.ai/api/v1/chat/completions` stream SSE; models from `GET /api/v1/models`. Installed = API key configured.
- HTTP providers are stateless: core sends full thread history (user/assistant messages) each turn. Token economy: cap history by configurable char budget (default 24k chars), oldest dropped first; pure fn, tested.
- Thread gets `model TEXT NULL` (migration). Required for HTTP providers; CLI providers use their own default unless set.
- `GET /providers/:kind/models` -> `[{id, name}]`.
- Tests: parse functions on recorded/handwritten NDJSON & SSE samples; axum test spins a fake Ollama/OpenRouter server (wiremock-style via axum on random port).

## Settings
`settings.json` in data dir, file mode 0600, owned by `octo-core::config`. Fields: `ollama_url`, `openrouter_api_key`, `history_char_budget`. API: `GET /settings` (key masked: `"sk-...abcd"`), `PATCH /settings`. Key never returned in full, never logged.

## Spaces
- Policy unchanged: private => local providers only (Ollama). Enforced at create, send, and attach.
- Public space dir: `<data>/public/` — plain files. CLI agents in public threads get it as working dir context (`claude --add-dir`, `agy --add-dir`).
- Private space dir: `<data>/private/` — encrypted at rest. XChaCha20-Poly1305 (`chacha20poly1305` crate), random 24-byte nonce per file, file = nonce||ciphertext. Key: 32 random bytes in `<data>/keys/private.key` (0600), created on first use. (Passphrase / OS keyring later.) File names inside private dir are encrypted too: store an encrypted index `index.bin` (JSON tree of `{id, path, size, created_at}`), blobs named by random id.
- Never decrypt to disk. Decrypt in memory only when serving to the UI or attaching to a local-model prompt.

## Files API
| Method | Path | Notes |
|---|---|---|
| GET | /files/:space | tree `[{path, kind: file|dir, size, modified}]` |
| POST | /files/:space?path=dir/ | multipart upload (limit 50 MB) |
| GET | /files/:space/content?path= | download (decrypted for private) |
| POST | /files/:space/dir?path= | mkdir |
| PATCH | /files/:space?path= | `{new_path}` rename/move |
| DELETE | /files/:space?path= | file or empty dir |
Path validation: relative, no `..`, no absolute, normalized — pure fn with tests (traversal attempts).

## Attachments
`POST /threads/:id/messages` body `{content, attachments?: [{space, path}]}`. Attachment space must equal thread space (policy). Text files (utf-8, <=200 KB) are inlined into the prompt as fenced blocks with the path as header; images and PDFs: see `docs/stage6.md`; other types rejected with 415. Message stores attachment refs (`attachments TEXT JSON`).

## Web
- Right side panel, collapsible: file tree with two tabs `Private` / `Public` (VS Code-like: tree, drag-drop upload, context actions rename/delete/new folder, click = preview text). Active tab follows the active thread's space by default.
- Attach: drag a file from the tree into the input, or paperclip picker from current space. Chips above input.
- New thread button in the sidebar -> dialog: assistant cards (installed providers), for HTTP providers a searchable model list, space choice (Private disabled unless the provider is local), for CLI providers an "Ask before running commands" checkbox (off = approval `auto`).
- Settings: labelled Settings button at bottom of sidebar -> full-screen page with sections Models (Ollama URL + Check, OpenRouter key + Check, showing models count) and Network (Join or Create a network when the device is in none; otherwise the network, this device, the members with their permissions, Add device, and a menu with Rename network, Relay and Leave network).
- Thread list label: provider + model short name, muted.

## Deviations
- `ProviderKind` serializes as `"openrouter"` (not serde's default `open_router`), matching the DB value and the TS type.
- `GET /files/:space` returns a flat list ordered component-wise (a dir is followed by its children), not a nested tree; the client builds the tree. Entries carry `modified` (unix ms). The private `index.bin` additionally stores `kind` and `modified`.
- Private blobs are bound to their id as AEAD associated data (and `index.bin` to a fixed tag), so blobs cannot be swapped between files undetected. Key/dirs: `<data>/keys/private.key` is created on first private access, not at startup.
- Upload (`POST /files/:space?path=dir/`): each multipart file part is stored at `dir/<filename>`; the filename may itself contain `/` subpaths (validated). An existing file is overwritten; rename onto an existing path is 409; deleting a non-empty dir is 409. Parent dirs are created implicitly. The 50 MB limit is the request body limit (413).
- Public store never follows or lists symlinks; any symlink component in a requested path is rejected.
- Downloads are served as `application/octet-stream` with `nosniff` and `CSP: sandbox` so user files cannot run script in the app origin.
- Attachments are inlined only into the turn that carries them; earlier turns are re-sent as their plain message text (token economy). The stored message keeps `attachments` refs, the `content` stays the user's text.
- CLI providers: `--model` is passed when the thread has a model; `--add-dir <data>/public` only for public threads (both CLIs accept it; verified via `--help`, not run against the real services). Private threads never use CLI providers.
- OpenRouter streams that close without `[DONE]` are completed with `done` only after a `finish_reason` was seen; otherwise (and for an interrupted connection) the result is `error`. HTTP streams are cut with `error` after 120 s without data, and a line over 8 MB is an error. Ollama `done:true` lines may carry text, which is emitted first.
- A run is released (thread no longer busy) before the SSE stream closes, so a client may send the next message right after `done`.
- Upstream failures of `GET /providers/:kind/models` are `502`; CLI kinds return `[]`. `openrouter_base` is a `Config` field (tests point it at a fake server), not a user setting.
- Settings: keys shorter than 12 chars are masked as `...`. `PATCH` validates the whole body before applying; `openrouter_api_key: ""` clears the key.
- File I/O in handlers (and attachment reads) runs in `tokio::task::spawn_blocking`.
- Ollama counts as local only if `ollama_url` has a loopback host (127.0.0.0/8, `::1`, `localhost`) or a private/link-local IP literal (10/8, 172.16/12, 192.168/16, 169.254/16, fc00::/7, fe80::/10); any other host makes private threads refuse it (policy error at create and send). `ollama_url` and `relay_url` are parsed as URLs (scheme, host, no credentials).
- Stored enum values (space, provider, role, approval, origin) and a newer database `user_version` are errors, never defaulted.
- The user message is stored before the provider is spawned; if the start fails it is removed again. A cancel that arrives between registering a run and the spawn is applied right after the spawn.
