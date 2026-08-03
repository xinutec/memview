#!/usr/bin/env bash
# The agent console on the Mac: drive the live Claude Code sessions.
#
#   ./scripts/console.sh
#
# Loopback by default, and that is not a dev convenience — it is the security
# model. A process already running as this user can spawn `claude` itself and
# gains nothing by asking us, so loopback needs no authentication; the home LAN
# holds a dozen unpatched devices and gets none of that benefit of the doubt.
#
# It binds off loopback only once there is something to authenticate *with*: a key
# of its own (scripts/console-identity.sh) and at least one pinned client
# (scripts/enrol.sh). The runner refuses to do it otherwise — see
# docs/agent-console.md and console/src/tls.rs.
#
# CONSOLE_PERMISSION_MODE is left unset on purpose. Under the CLI's default a
# headless session is refused every tool call that needs permission, so it can
# converse and little else — measured, not assumed. `acceptEdits` is the setting
# that makes a session useful in a directory you trust; nothing here chooses that
# for you.
set -euo pipefail
cd "$(dirname "$0")/.."

DIR="${CONSOLE_HOME:-$HOME/.config/agent-console}"

if [ -s "$DIR/server.key" ] && [ -s "$DIR/clients" ]; then
  # One pin per line in the file so it can be commented and edited by a person;
  # commas on the way out because that is what the runner parses.
  PINS="$(grep -v '^[[:space:]]*\(#.*\)\?$' "$DIR/clients" | paste -sd, -)"
  echo "gate: $(printf '%s\n' "$PINS" | tr ',' '\n' | wc -l | tr -d ' ') pinned client key(s)"
  echo "desk: http://${CONSOLE_DESK_ADDR:-127.0.0.1:8096} — the gated socket asks everybody"
  echo "      for a certificate, an SSH forward from here included, so this machine"
  echo "      keeps its own way in."
  export CONSOLE_TLS_CERT="$DIR/server.crt"
  export CONSOLE_TLS_KEY="$DIR/server.key"
  export CONSOLE_CLIENT_KEYS="$PINS"
  # Loopback, even with the gate on. The phone arrives through a tunnel this Mac
  # dialled out to isis (scripts/console-tunnel.sh), so the socket never has to be
  # reachable from anywhere — and a machine nothing can connect to is a stronger
  # statement than a machine that answers one address. See docs/agent-console.md.
  export BIND_ADDR="${BIND_ADDR:-127.0.0.1:8097}"
  # The tunnel lives exactly as long as the console does. Started here rather than
  # as a launchd agent because a standing tunnel to a console that is not running
  # is a listening port on isis with nothing behind it — and because the thing and
  # the thing watching it should stop together.
  ./scripts/console-tunnel.sh &
  TUNNEL=$!
  trap 'kill "$TUNNEL" 2>/dev/null || true' EXIT
else
  echo "gate: not configured — loopback only (scripts/console-identity.sh sets it up)"
fi

# Not `exec`: the trap above has to survive to take the tunnel down with us.
CONSOLE_DIRS="${CONSOLE_DIRS:-$HOME/Code}" \
  # Served from console-live, NOT from the build output. `ng build` deletes its
  # whole output path, so a bundle being served from there vanishes for a second
  # on every build — and a page reloading in that second gets HTML where it
  # asked for a font, which shows as broken icons and is reported by nothing.
  # `build:console` rsyncs into console-live without deleting, so the previous
  # build`s hashed files stay behind and a page mid-load still finds its own.
  STATIC_DIR="${STATIC_DIR:-frontend/dist/console-live}" \
  nix develop -c cargo run -p console
