#!/bin/sh
# Dev: API server on 127.0.0.1:7878 plus Vite dev server (proxies /api).
set -e
cd "$(dirname "$0")/.."
cargo build -p octo-server
target/debug/octo-server "$@" &
server=$!
trap 'kill $server 2>/dev/null' EXIT INT TERM
cd web && npm run dev
