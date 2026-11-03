# Prototype 0

Goal: prove Rust core is viable. Chat with Claude Code and Antigravity CLI from a web UI, streamed, persisted in SQLite, cancellable.

## Layout
```
Cargo.toml              workspace
crates/octo-core        lib: config, storage, providers, process registry, policy
crates/octo-server      bin: axum HTTP on 127.0.0.1:7878, serves web/dist in release
web/                    Vite + React 19 + TS + Tailwind (base taken from old tasktree frontend)
tests/fixtures/         fake CLI scripts emitting recorded stream-json
```
Later `octo-core` is embedded directly in Tauri (no sidecar).

## Data
Data dir: `directories::ProjectDirs::from("", "", "octo")` data dir, overridable by `--data-dir`. DB: `octo.db`.

```sql
threads(id TEXT PK, title TEXT, provider TEXT, space TEXT CHECK(space IN('private','public')),
        external_session_id TEXT, created_at INTEGER, updated_at INTEGER)
messages(id TEXT PK, thread_id TEXT FK, role TEXT CHECK(role IN('user','assistant','error')),
         content TEXT, created_at INTEGER)
```
Migrations: embedded SQL, `PRAGMA user_version`.

## Providers
`enum ProviderKind { Claude, Antigravity }` (Ollama/OpenRouter next). Each provider:
- `detect() -> ProviderStatus { installed, version }` (binary in PATH; path configurable)
- `is_local() -> bool` (both CLIs: false)
- `run(prompt, external_session_id: Option) -> stream of AgentEvent`

```rust
enum AgentEvent { Session{id}, TextDelta{text}, Tool{name, summary}, Done, Error{message} }
```
- Claude: `claude -p --output-format stream-json --verbose --include-partial-messages` + `--session-id <uuid>` for new threads (we generate the uuid) or `--resume <id>`. Prompt on stdin.
- Antigravity: `agy -p --output-format stream-json` + `--conversation <id>` when resuming; session id taken from stream. Prompt on stdin (verify; if unsupported, document and use the least-bad alternative).
- Parsing is a pure function `parse_line(&str) -> Vec<AgentEvent>` per provider, unit-tested against recorded fixture lines in `tests/fixtures/`. Record fixtures from one real short call each.

## Policy
`fn check_provider(space, provider) -> Result<(), PolicyError>`: private + non-local => error. Enforced in `send_message` before spawn.

## HTTP API (JSON, prefix /api)
| Method | Path | Body / Result |
|---|---|---|
| GET | /providers | `[{kind, installed, version, is_local}]` |
| GET | /threads | `[Thread]` newest first |
| POST | /threads | `{provider, space}` -> `Thread` |
| DELETE | /threads/:id | 204 |
| GET | /threads/:id/messages | `[Message]` |
| POST | /threads/:id/messages | `{content}` -> `text/event-stream` of `AgentEvent` (JSON per `data:`), event names = variant in snake_case |
| POST | /threads/:id/cancel | 204, kills running process |

Thread: `{id, title, provider, space, created_at, updated_at}`. Message: `{id, thread_id, role, content, created_at}`. Title = first 60 chars of first user message. On Done/Error the assistant (or error) message is persisted. One run per thread at a time (409 otherwise).
TS types generated with `ts-rs` into `web/src/api/types/`.

## Web UI
- Left: thread list, one "+" button with dropdown of installed providers, space toggle (private/public) on creation.
- Center: message feed (markdown), input at bottom, stop button while streaming.
- Dark theme by default; reuse token/theme approach from old `frontend/src/index.css` + `tailwind.config.js` (drop vogue palette).
- Dev: Vite proxy `/api` -> `127.0.0.1:7878`.

## Done when
`cargo test` + `npm test` green; manual: create Claude thread, send, see streamed answer, reload page, history persists, follow-up message resumes same session, cancel kills process (no orphan in `ps`).
