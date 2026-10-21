#!/usr/bin/env bash
# The agent console as a launchd service. Entry point for org.xinutec.agent-console.
#
# `console.sh` runs one by hand; this is the supervised copy, shaped by launchd:
#
# - Nothing here touches nix: `nix-shell` hangs under launchd. Building is
#   `console-upgrade.sh`'s job, from a terminal.
# - It runs the installed binary, not the cargo target, so `cargo clean` is not
#   an outage.
# - It `exec`s, so the job's pid is the console: the SIGUSR2 handover keeps that
#   pid and launchd never sees an upgrade. Never `launchctl kickstart -k`: its
#   SIGTERM stops every session.
# - The tunnel stays in this job's process group, which launchd kills when the
#   job dies; a trap cannot run once the shell has exec'd. A tunnel with no
#   console behind it is an open port on isis.
# - It runs from its installed copy in ~/.local/libexec: under launchd, Apple's
#   binaries are refused the external volume the checkout is on (`/bin/bash`
#   reading a script there exits 126, with an empty log, as the log is on the
#   volume too). Only the console binary, not an Apple one, opens the checkout,
#   for `$STATIC_DIR`.
set -euo pipefail

# The install directory under launchd, `scripts/` by hand; never `..`, as the
# installed copy has no repo above it.
HERE="$(cd "$(dirname "$0")" && pwd)"

CONSOLE_BIN="${CONSOLE_BIN:-$HOME/.local/libexec/agent-console}"
DIR="${CONSOLE_HOME:-$HOME/.config/agent-console}"
REPO="${CONSOLE_REPO:-$HOME/Code/memview}"

if [ ! -x "$CONSOLE_BIN" ]; then
  echo "no console binary at $CONSOLE_BIN — run scripts/console-upgrade.sh once" >&2
  exit 78 # EX_CONFIG: a configuration problem, not a crash to be retried hard.
fi

if [ -s "$DIR/server.key" ] && [ -s "$DIR/clients" ]; then
  PINS="$(grep -v '^[[:space:]]*\(#.*\)\?$' "$DIR/clients" | paste -sd, -)"
  echo "gate: $(printf '%s\n' "$PINS" | tr ',' '\n' | wc -l | tr -d ' ') pinned client key(s)"
  export CONSOLE_TLS_CERT="$DIR/server.crt"
  export CONSOLE_TLS_KEY="$DIR/server.key"
  export CONSOLE_CLIENT_KEYS="$PINS"
  export BIND_ADDR="${BIND_ADDR:-127.0.0.1:8097}"
  # Not waited on (see the process-group note above); it redials by itself.
  "$HERE/console-tunnel.sh" &
else
  echo "gate: not configured — loopback only (scripts/console-identity.sh sets it up)"
fi

export CONSOLE_DIRS="${CONSOLE_DIRS:-$HOME/Code}"
# console-live, not the build output: `ng build` deletes its output path, so a
# page loading mid-build would get HTML for a font. Only `pnpm run
# publish:console` writes there.
export STATIC_DIR="${STATIC_DIR:-$REPO/frontend/dist/console-live}"

exec "$CONSOLE_BIN"
