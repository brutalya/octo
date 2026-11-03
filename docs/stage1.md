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
