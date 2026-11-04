# Stage 2 — import and continue existing CLI sessions

Goal: sessions started in a terminal (Antigravity, Claude Code) appear in Octo and can be continued; sessions continued in Octo can still be continued in the terminal.

## Model
- Source of truth for CLI threads is the CLI's own storage. Octo keeps a link (`external_session_id`) and a message cache.
- Thread gets `cwd TEXT NULL` (migration). CLI runs use it as working dir (both CLIs scope sessions by project dir). New Octo CLI threads: cwd = `<data>/public` unless set.
- Thread gets `origin TEXT CHECK(origin IN('octo','imported'))`.
- Imported threads are always `public` space.
- On open (`GET /threads/:id/messages`) of a CLI thread: if the source session changed since last sync (mtime / step count), re-read and replace the cache. Pure diff not needed — replace.

## Readers (pure, read-only, tested on fixtures copied/trimmed from real data)
- **Claude Code**: `~/.claude/projects/<encoded-cwd>/<session-id>.jsonl`. List: id, cwd, title (first user text, 60 chars), updated_at (mtime), message count. Messages: user/assistant text blocks only; tool calls -> `Tool` summary line; skip meta/sidechain.
- **Antigravity**: index from `~/.gemini/antigravity-cli/conversation_summaries.db` (title, step_count, last_modified_time, workspace_uris). Messages — first working option in this order, document which one was used:
  1. an official way (`agy` subcommand / `agy acp` `session/load` returning history),
  2. decode `conversations/<id>.db` `steps.step_payload` (protobuf without schema: extract user input and model response text fields heuristically; open DB read-only, `immutable=1`),
  3. fallback: user prompts from `history.jsonl` + title; assistant side shown as one muted line "history unavailable".
- Paths come from config (overridable), never hardcoded in readers. Open SQLite read-only; never write to CLI storage.

## API
| Method | Path | Result |
|---|---|---|
| GET | /import/:kind/sessions?q= | `[{external_id, title, cwd, updated_at, messages, imported_thread_id?}]` newest first |
| POST | /import/:kind | `{external_id}` -> Thread (idempotent: returns existing thread if already imported) |

## Web
- "+" dropdown: under each CLI provider a second item opens a searchable session list (title, cwd short, relative time). Click = import + open.
- Imported threads look like any other thread. No badges.
