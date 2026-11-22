# Octo — rules for agents

Octo is an AI interaction hub: chats with CLI agents (Claude Code, Antigravity), local models and OpenRouter, with strict Private / Public spaces and (later) E2EE sync between PC, server and phone. Everything else (tasks, kitchen, ...) is a plugin, not core.

Source of truth: this file + `docs/`. The old project `tasktree` is reference only — never copy its architecture, never import its concepts (Sovereign OS, Knowledge Lake, Inspector, Sandbox, Mem0, modes).

## Engineering
- Tests first: write the failing test, then the implementation. Never edit a test just to make code pass.
- Small modules, boring code. Rust: tokio, axum, rusqlite (bundled), serde, thiserror/anyhow. No custom trait hierarchies, no heavy generics, no `unsafe`, no `unwrap()` outside tests.
- One source of truth for config: `octo-core::config`. No env-var mode switching, no `cfg(test)` behaviour branches in production paths.
- Every child process is owned by `ProcessRegistry`, spawned with `kill_on_drop`, cancellable, reaped. No fire-and-forget processes.
- Prompts go to CLIs via stdin, never argv.
- Auto-approval flags only when the thread's approval mode is `auto`, explicitly chosen by the user; default is `confirm`.
- Server binds `127.0.0.1` by default.
- `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test`, `npm test`, `npm run build` must pass before commit.

## Privacy
- `space` is `private` or `public` on every thread. Private threads may only use providers where `Provider::is_local() == true`. Enforced in core (policy function + test), not in UI.
- Private data never leaves the device unencrypted. No cloud fallback, ever.

## UI
- Minimal. No decorative headers, no badges, no emoji, no placeholder "coming soon" features, no explanatory text that the UI already makes obvious.
- Show only configured/installed providers.
- Icons: lucide-react. No CDN assets (works offline).

## Docs
- README and `docs/deploy.md` must be updated in the same commit as any change to build, run, deploy, ports, flags or config.
- Docs are concise and complete.
- Docs never contain personal data (real domains, IPs, usernames, emails, keys, local paths of a specific machine); use placeholders (`relay.example.com`, `<owner>/<repo>`).

## Workflow
- `main` is production. Never commit or push to `main` directly: one branch per feature/fix/issue from `origin/main` (`feat/`, `fix/`, `docs/`, `ci/`, `test/`), checks pass locally, then a pull request.
- CI runs by changed paths (`ci.yml`); `ci-ok` is the merge gate. Docs-only changes run no tests.
- A release is a version bump (`tauri.conf.json` + `octo-desktop/Cargo.toml`) merged to `main`; `release.yml` tags and publishes it.

## Commits
Conventional commits, small and focused.
