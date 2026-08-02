#!/usr/bin/env bash
# The agent console on the Mac: drive the live Claude Code sessions.
#
#   ./scripts/console.sh
#
# Loopback only, and that is not a dev convenience — it is the security model
# until the client-certificate gate exists. The runner refuses to bind anywhere
# else, so this is reachable from a browser ON the Mac and from nothing else.
# The Mac being headless is exactly why: see docs/agent-console.md, phase 1.
#
# CONSOLE_PERMISSION_MODE is left unset on purpose. Under the CLI's default a
# headless session is refused every tool call that needs permission, so it can
# converse and little else — measured, not assumed. `acceptEdits` is the setting
# that makes a phase-1 session useful in a directory you trust; nothing here
# chooses that for you.
set -euo pipefail
cd "$(dirname "$0")/.."

CONSOLE_DIRS="${CONSOLE_DIRS:-$HOME/Code}" \
  STATIC_DIR="${STATIC_DIR:-frontend/dist/console-web/browser}" \
  exec nix develop -c cargo run -p console
