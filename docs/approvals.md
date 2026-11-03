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
