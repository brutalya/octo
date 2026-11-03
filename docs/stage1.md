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
`POST /threads/:id/messages` body `{content, attachments?: [{space, path}]}`. Attachment space must equal thread space (policy). Text files (utf-8, <=200 KB) are inlined into the prompt as fenced blocks with the path as header; other types rejected with 415 for now. Message stores attachment refs (`attachments TEXT JSON`).

## Web
- Right side panel, collapsible: file tree with two tabs `Private` / `Public` (VS Code-like: tree, drag-drop upload, context actions rename/delete/new folder, click = preview text). Active tab follows the active thread's space by default.
- Attach: drag a file from the tree into the input, or paperclip picker from current space. Chips above input.
- "+" dropdown: providers -> for HTTP providers a model submenu (searchable list). Private option shown only for local providers.
- Settings: gear at bottom of sidebar -> modal with one section per provider (Ollama URL + check button showing models count; OpenRouter key + check). Nothing else.
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
- OpenRouter streams that close without `[DONE]` are completed with `done`; an interrupted connection yields `error`. Ollama `done:true` lines may carry text, which is emitted first.
- A run is released (thread no longer busy) before the SSE stream closes, so a client may send the next message right after `done`.
- Upstream failures of `GET /providers/:kind/models` are `502`; CLI kinds return `[]`. `openrouter_base` is a `Config` field (tests point it at a fake server), not a user setting.
- Settings: keys shorter than 12 chars are masked as `...`. `PATCH` validates the whole body before applying; `openrouter_api_key: ""` clears the key.
- File I/O in handlers is synchronous (small local files, single user); moving it to `spawn_blocking` is a later optimization.
