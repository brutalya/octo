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
  2. decode `conversations/<id>.db` `steps.step_payload` (protobuf without schema: extract user input and model response text fields heuristically; open DB read-only),
  3. fallback: user prompts from `history.jsonl` + title; assistant side shown as one muted line "history unavailable".

  Used: option 2, with option 3 as the per-conversation fallback (see Deviations).
- Paths come from config (overridable), never hardcoded in readers. Open SQLite read-only; never write to CLI storage.

## API
| Method | Path | Result |
|---|---|---|
| GET | /import/:kind/sessions?q= | `[{external_id, title, cwd, updated_at, messages, imported_thread_id?}]` newest first |
| POST | /import/:kind | `{external_id}` -> Thread (idempotent: returns existing thread if already imported) |

## Web
- New chat dialog: with a CLI provider chosen, `Continue a <provider> session` opens a searchable session list (title, cwd short, relative time). Click = import + open. Sessions started outside Octo and not imported yet are also listed in All chats in the sidebar.
- Imported threads look like any other thread. No badges.

## Deviations
- **Antigravity messages: option 2, option 3 as fallback.** `agy` has no history export (`--conversation <id>` resumes, `-p` prints one turn), so `steps.step_payload` is decoded with a small built-in protobuf wire reader (varint and length-delimited fields only). Layout, found by inspecting real sessions (no official schema, may change with CLI versions): payload field `1` is the step type; type 14 is a user turn with the prompt in `19.2`; type 15 is a model reply with the text in `20.1`; `5.1` holds the timestamp (`1` seconds, `2` nanos). Other steps (tool calls, system notices) are skipped and consecutive replies are merged into one assistant message. The database is opened read-only, never written. If `conversations/<id>.db` is missing, unreadable or yields no text, the thread falls back to the user prompts from `history.jsonl` (entries with `type` null, i.e. not slash commands) plus one assistant line `*history unavailable*`; `history.jsonl` has no conversation id, so a prompt is assigned to a conversation by workspace and time: the first session of that workspace whose `last_modified_time` is at or after the prompt, and after the previous session of the same workspace. Sub-agent conversations (`parent_conversation_id` set) and conversations with 0 steps are not listed.
- **Cache refresh only for imported threads.** Octo-created CLI threads keep their own cache (the CLI copy contains inlined attachment text). Imported Claude threads refresh when the stamp (`mtime:size`) changes. Imported Antigravity threads refresh only from a decoded `conversations/<id>.db`; the `history.jsonl` fallback is lossy and never replaces a cache, so a cache that holds only `*history unavailable*` (imported before decoding worked or before the db was readable) is replaced as soon as the steps can be read (decoded sessions carry a distinct stamp suffix). Refresh is skipped while a run is in progress; a vanished or undecodable source keeps the cache.
- Claude tool calls become `> Tool: <name> <summary>` lines inside the assistant message; consecutive assistant entries are merged into one message. Envelope user entries (`<command-...>`, `<local-command-...>`, `<system-reminder>`), thinking blocks, sidechain and meta entries are skipped.
- `cwd` is a normal field of `POST /threads`; CLI threads default to `<data>/public`. A run fails fast (400, nothing stored) if the working directory does not exist.
- Config: `claude_projects_dir` and `agy_dir` default to `<home>/.claude/projects` and `<home>/.gemini/antigravity-cli` (home via the platform home dir, so `%USERPROFILE%` on Windows); server flags `--claude-projects-dir`, `--agy-dir`.
- Session ids from the API are validated (`[A-Za-z0-9_-]`) before any file lookup.
- Web: in the New chat dialog, with a CLI provider chosen, `Continue a <provider> session` opens the search list (server-side `q`, debounced).
- CLI agents are not sandboxed: in `auto` approval they can read and write the whole user account, whatever the working directory. Octo only refuses a thread `cwd` that is, or is inside, the data directory (canonicalized), except the public space dir, so the CLI does not start next to private blobs, keys or the database.
- After a terminal event the CLI process gets 2 s to exit, then it is cancelled. Stderr kept for error messages is capped at 64 KB. Claude stream events from sub-agents (`parent_tool_use_id` set) are not shown as reply text.

