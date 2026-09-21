#!/usr/bin/env bash
# The two-device draft sync suite: a REAL runner, two browser contexts, and a
# draft crossing between them.
#
# Its own row rather than part of `ui-check`. That harness serves the
# bundle itself and stubs `/api/**`, which is right for phone-width layout and
# is exactly why it has never exercised sync. This one needs the opposite: the
# Rust runner serving both the app and `/api/sync/drafts`.
#
# Builds the binary rather than hoping for one. A debug build, because what is
# under test is the wire contract and not the runner's speed — and because the
# alternative is a test that passes against whatever was left in target/.
#
# And the BUNDLE, for exactly the same reason. The runner serves
# `dist/console-build/browser` and only refuses when there is nothing there at
# all, so it happily served a bundle built for something else — measured: the
# clash resolver's join order was reversed on purpose and all six tests stayed
# green, because the edit never reached the file being served. Rebuilding first
# turned that ablation red, which is the only way any of this is evidence. The
# `ui-check` row has the same trap and its own note
# (`reference_memview_ui_check_serves_the_built_dist`).
set -euo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

cargo build --quiet -p console --bin console
pnpm --dir frontend run build:console
CONSOLE_BIN="$PWD/target/debug/console" exec pnpm --dir frontend run sync-check
