#!/bin/sh
# Fake claude CLI that answers with its own working directory.
case "$*" in *--version*) echo "9.9.9 (fake)"; exit 0 ;; esac
cat >/dev/null
p=$(pwd -P)
printf '{"type":"system","subtype":"init","session_id":"s-pwd"}\n'
printf '{"type":"stream_event","event":{"delta":{"type":"text_delta","text":"%s"}}}\n' "$p"
printf '{"type":"result","is_error":false}\n'
