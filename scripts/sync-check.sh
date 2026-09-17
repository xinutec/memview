#!/usr/bin/env bash
# The two-device draft sync suite: a REAL runner, two browser contexts, and a
# draft crossing between them.
#
# ⚠ **Its own row rather than part of `ui-check`.** That harness serves the
# bundle itself and stubs `/api/**`, which is right for phone-width layout and
# is exactly why it has never exercised sync. This one needs the opposite: the
# Rust runner serving both the app and `/api/sync/drafts`.
#
# Builds the binary rather than hoping for one. A debug build, because what is
# under test is the wire contract and not the runner's speed — and because the
# alternative is a test that passes against whatever was left in target/.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

cargo build --quiet -p console --bin console
CONSOLE_BIN="$PWD/target/debug/console" exec pnpm --dir frontend run sync-check
