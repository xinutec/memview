#!/usr/bin/env bash
# The two-device draft sync suite: a REAL runner, two browser contexts, and a
# draft crossing between them.
#
# Its own row: `ui-check` stubs `/api/**`, so it never exercises sync; this needs
# the Rust runner serving both the app and `/api/sync/drafts`.
#
# Builds the binary and the bundle first. The runner serves whatever bundle is in
# `dist/console-build/browser`, so a deliberately broken edit once stayed green
# against an old build.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

cargo build --quiet -p console --bin console
pnpm --dir frontend run build:console
CONSOLE_BIN="$PWD/target/debug/console" exec pnpm --dir frontend run sync-check
