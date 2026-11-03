#!/bin/sh
# cargo test regenerates the ts-rs types; they must match what is committed.
set -e
cd "$(dirname "$0")/.."
cargo test -p octo-core
git diff --exit-code -- web/src/api/types
test -z "$(git ls-files --others --exclude-standard web/src/api/types)" || { echo "untracked generated types" >&2; exit 1; }
