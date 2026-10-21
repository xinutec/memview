#!/usr/bin/env bash
# The agent console on the Mac: drive the live Claude Code sessions.
#
#   ./scripts/console.sh
#
# Loopback by default, as the security model: a process running as this user
# could spawn `claude` itself, so loopback needs no authentication. The runner
# binds wider only with its own key (console-identity.sh) and a pinned client
# (enrol.sh); see docs/agent-console.md.
#
# CONSOLE_PERMISSION_MODE is unset on purpose: under the CLI's default a headless
# session is refused every tool call needing permission. `acceptEdits` is the
# user's choice to make.
set -euo pipefail
cd "$(dirname "$0")/.."

DIR="${CONSOLE_HOME:-$HOME/.config/agent-console}"

if [ -s "$DIR/server.key" ] && [ -s "$DIR/clients" ]; then
  # One pin per line for people, commas for the runner.
  PINS="$(grep -v '^[[:space:]]*\(#.*\)\?$' "$DIR/clients" | paste -sd, -)"
  echo "gate: $(printf '%s\n' "$PINS" | tr ',' '\n' | wc -l | tr -d ' ') pinned client key(s)"
  echo "desk: http://${CONSOLE_DESK_ADDR:-127.0.0.1:8096} — the gated socket asks everybody"
  echo "      for a certificate, an SSH forward from here included, so this machine"
  echo "      keeps its own way in."
  export CONSOLE_TLS_CERT="$DIR/server.crt"
  export CONSOLE_TLS_KEY="$DIR/server.key"
  export CONSOLE_CLIENT_KEYS="$PINS"
  # Loopback even with the gate on: the phone arrives through the tunnel this Mac
  # dials out to isis, so nothing need reach this socket.
  export BIND_ADDR="${BIND_ADDR:-127.0.0.1:8097}"
  # The tunnel stops with the console: one with nothing behind it is an open port
  # on isis.
  ./scripts/console-tunnel.sh &
  TUNNEL=$!
  trap 'kill "$TUNNEL" 2>/dev/null || true' EXIT
else
  echo "gate: not configured — loopback only (scripts/console-identity.sh sets it up)"
fi

# Not `exec`: the trap must survive to stop the tunnel. console-live, not the
# build output: `ng build` deletes its output path, so a page loading mid-build
# would get HTML for a font. `publish:console` copies there without deleting, so
# a page mid-load still finds its old hashed files; a plain build cannot reach
# it. scripts/console-serving.sh shows what is served.
CONSOLE_DIRS="${CONSOLE_DIRS:-$HOME/Code}" \
  STATIC_DIR="${STATIC_DIR:-frontend/dist/console-live}" \
  nix develop -c cargo run -p console
