# Interactive approvals (research, not implemented)

Today `confirm` = non-interactive deny: claude `--permission-mode default` and agy `toolPermission=request-review` auto-deny anything needing a prompt in print mode (agy lists them in `result.denied_actions`; claude in `result.permission_denials`). Goal: ask the user in the UI instead.

## Claude: `--permission-prompt-tool`
`claude -p ... --permission-prompt-tool mcp__octo__approve --mcp-config <json> --strict-mcp-config`. Claude calls this MCP tool whenever a tool use needs permission; the tool returns JSON text:
- in: `{tool_name, input, tool_use_id}`
- out: `{"behavior":"allow","updatedInput":<input>}` or `{"behavior":"deny","message":"..."}`
(`--permission-prompts host` is the default in -p, so no extra flag.)

Hosting the MCP tool, options:
1. HTTP/SSE MCP endpoint inside octo-server (`/mcp/<run_token>`), passed via `--mcp-config '{"mcpServers":{"octo":{"type":"http","url":"http://127.0.0.1:PORT/mcp/<token>"}}}'`. No extra process, token = per-run, maps the call to thread id. Needs a minimal MCP server (initialize, tools/list, tools/call; ~150 lines JSON-RPC, or the `rmcp` crate). Preferred.
2. stdio MCP: a tiny `octo-mcp` bin spawned by claude (not by us, so outside ProcessRegistry; violates the rule). Avoid.

Flow: tools/call arrives -> hub registers `PendingApproval{id, thread_id, tool, input}` and emits a new `AgentEvent::ApprovalRequest{id, tool, summary}` on the thread SSE -> UI shows Allow/Deny -> `POST /threads/:id/approvals/:id {allow:bool}` resolves a oneshot -> MCP response returned. Timeout/cancel/disconnect => deny. `auto` mode uses Claude `auto` permission mode (classifier-based auto approval) and skips this.

Alternative: `--input-format stream-json` keeps one long-lived process and a persistent stdin (also enables multi-turn without `--resume` respawn), but permission decisions still go through the prompt tool; control messages (`control_request`/`can_use_tool` in the SDK protocol) are SDK-internal and undocumented for the CLI. Not recommended as the first step.

## agy
- `agy acp`: Agent Client Protocol stdio server (JSON-RPC 2.0, `session/new`, `session/prompt`, `session/update`, and the standard `session/request_permission` request with `PermissionOption`s; strings for these are in the binary). This is the real equivalent of the prompt tool: the client (octo) answers `request_permission` with a chosen option id. Needs a spike: record a real session, confirm tool_call events and the permission payload. Replaces the `-p` stream-json path for agy in `confirm` mode.
- `--input-format stream-json` + `-p`: no permission request/response events exist (only `denied_actions` after the fact). Cannot approve interactively.
- `--remote-control` / `agy remote-control`: registers a daemon with Google's cloud relay; not suitable (cloud dependency, not a local API).
- `bin/agentapi`: wrapper for `agy agentapi` (new-conversation, send-message, get-conversation-metadata): multi-agent messaging, no approval hook.
- Fallback without ACP: replay denied action. After a run ends with `denied_actions`, show them, and on "Allow" re-run the turn with `--settings` permissions.allow rule (e.g. `command(<target>)`) for that action only. Coarse, wasteful, but cheap.

## Common design
- `Approval` stays per thread; add a per-run `ApprovalBroker` in octo-core (pending map, oneshot per request, cancel clears all).
- New SSE event `approval_request`, new endpoint `POST /threads/:id/approvals/:id`; the event persists in the message feed as a tool row with its decision.
- UI: inline card under the streaming message, Allow / Deny (+ "allow for this thread" later via claude `updatedPermissions`).

## Effort
- Claude via HTTP MCP: ~1.5-2 days (MCP endpoint + broker + SSE/REST + UI card + fake-CLI fixture tests with a scripted MCP client).
- agy via ACP: ~3-4 days (spike 0.5d, new ACP client provider with its own event mapping, session resume via ACP, tests).
- Fallback-replay for agy: ~0.5 day.
Risk: claude -p as root refuses bypassPermissions (needs `IS_SANDBOX=1`), so octo uses Claude `auto` mode for `auto` approval instead.

## Spec, step 1: Claude (decided)
- `confirm` for Claude becomes interactive; `auto` unchanged. agy `confirm` stays non-interactive deny until step 2.
- MCP endpoint in octo-server: `POST /mcp/:run_token`, minimal JSON-RPC over Streamable HTTP with single JSON responses (`initialize`, `notifications/initialized` -> 202, `tools/list`, `tools/call`; anything else -> JSON-RPC error). One tool `approve` (input `{tool_name, input, tool_use_id}`). No new dependency.
- Auth: `run_token` = 32 random bytes hex, one per run, compared in constant time, invalid after the run ends. The route is outside the bearer-token layer, refused through the tunnel (not in the allow-list) and subject to the Host check.
- Claude gets `--permission-prompt-tool mcp__octo__approve --mcp-config <json> --strict-mcp-config` (config JSON in argv is fine: no prompt, no secret beyond the per-run token). The URL uses the actually bound API address: the server binds first, then builds the hub with `Config.local_api` set (one config source).
- Core `ApprovalBroker` (per hub): `request(thread, tool, input) -> oneshot<bool>`, `resolve(thread, id, allow) -> bool`, `cancel_thread(thread)` (deny all). Timeout 10 min -> deny. Run cancel/finish -> deny pending.
- `AgentEvent::Approval {id, tool, summary}` on the thread SSE (summary = tool name + short input: command / file path / url, max 200 chars). Decision recorded in the assistant message as `> Allowed: <summary>` / `> Denied: <summary>`.
- `POST /api/threads/:id/approvals/:aid {allow: bool}` -> 204, 404 if unknown. Through the tunnel it requires the `agents` permission (policy + test).
- SSE responses send a keep-alive comment every 15 s (mux idle timeout is 60 s; waiting for a decision is silent).
- `GET /api/threads/:id/approvals` -> pending list, so a reopened UI (or another device) sees open requests.
- UI: inline row under the streaming message: summary + Allow (Check) / Deny (X) icon buttons, no text. Disappears when resolved.
- Tests first: MCP protocol unit tests (initialize, tools/list, tools/call allow/deny, bad token 401/404, after-run token rejected); broker (resolve, timeout, cancel); fake-claude fixture script that calls the MCP endpoint with curl-free Rust? -> use a small test helper acting as the MCP client plus the existing fake CLI; tunnel policy; web vitest for the approval row.

### Deviations from the spec (as built)
- The run token exists only for runs that get the MCP flags (Claude, `confirm`, `Config.local_api` set). Other runs have none.
- `CliExtra.mcp_url` carries the endpoint into the argument builder (no extra parameter).
- Decisions are injected into the run's event stream and saved as part of the assistant text, as `> Allowed: <summary>` / `> Denied: <summary>` on their own lines.
- `GET /api/threads/:id/approvals` also requires the `agents` permission through the tunnel.
- A wildcard bind address (`0.0.0.0`) is dialed on loopback by Claude.
- The UI loads open requests when a thread is opened; it learns about later ones only from its own stream.
- E2E tests use `tests/fixtures/fake_claude_mcp.sh` (bash `/dev/tcp`, unix only) instead of a Rust helper.
