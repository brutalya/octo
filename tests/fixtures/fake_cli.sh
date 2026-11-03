#!/bin/sh
# Fake provider CLI: replays recorded fixture lines. Prompt arrives on stdin.
# If the prompt contains SLOW, emit one line and then hang until killed.
dir=$(dirname "$0")
case "$*" in *--verbose*) p=claude ;; *) p=antigravity ;; esac
case "$*" in *--resume*|*--conversation*) k=resume ;; *) k=new ;; esac
case "$*" in *--version*) echo "9.9.9 (fake)"; exit 0 ;; esac
prompt=$(cat)
case "$prompt" in
  *FAIL*) echo "fake failure" >&2; exit 3 ;;
  *SLOW*) head -n 1 "$dir/${p}_$k.jsonl"; exec sleep 30 ;;
esac
cat "$dir/${p}_$k.jsonl"
