#!/usr/bin/env bash
# Fake Claude CLI that does the permission-prompt round trip like the real one: reads the
# --mcp-config argument, calls the `approve` tool of that MCP server over plain HTTP
# (bash /dev/tcp, no curl) and reports the decision as streamed text. Prompt arrives on stdin.
case "$*" in *--version*) echo "9.9.9 (fake)"; exit 0 ;; esac
cat >/dev/null
text() {
  printf '{"type":"stream_event","parent_tool_use_id":null,"event":{"delta":{"type":"text_delta","text":"%s"}}}\n' "$1"
}
cfg=""
while [ $# -gt 0 ]; do
  [ "$1" = "--mcp-config" ] && cfg=$2
  shift
done
echo '{"type":"system","subtype":"init","session_id":"fake-session"}'
if [ -z "$cfg" ]; then
  text "NOMCP"
  echo '{"type":"result","is_error":false}'
  exit 0
fi
url=$(printf '%s' "$cfg" | sed -n 's/.*"url":"\([^"]*\)".*/\1/p')
rest=${url#http://}
hostport=${rest%%/*}
path=/${rest#*/}
host=${hostport%:*}
port=${hostport##*:}
rpc() {
  exec 3<>"/dev/tcp/$host/$port" || return 1
  printf 'POST %s HTTP/1.1\r\nHost: %s\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: %d\r\nConnection: close\r\n\r\n%s' \
    "$path" "$hostport" "${#1}" "$1" >&3
  cat <&3
  exec 3<&-
}
rpc '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"fake","version":"0"}}}' >/dev/null
rpc '{"jsonrpc":"2.0","method":"notifications/initialized"}' >/dev/null
reply=$(rpc '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"approve","arguments":{"tool_name":"Bash","input":{"command":"ls"},"tool_use_id":"toolu_1"}}}')
if printf '%s' "$reply" | grep -q '\\"behavior\\":\\"allow\\"'; then
  text "RESULT:ALLOWED"
elif printf '%s' "$reply" | grep -q '\\"behavior\\":\\"deny\\"'; then
  text "RESULT:DENIED"
else
  text "RESULT:BROKEN"
fi
echo '{"type":"result","is_error":false}'
