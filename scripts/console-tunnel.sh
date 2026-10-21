#!/usr/bin/env bash
# The console's way in, dialled from this side.
#
#   ./scripts/console-tunnel.sh
#
# isis listens on its VPN address and passes what arrives down this connection to
# the console on loopback. Nothing connects toward this Mac, so the one-way VPN
# rule stands.
#
# isis carries only ciphertext: TLS ends at the console's own gate, so isis can
# drop the tunnel but not read, inject or impersonate.
#
# The key may do one thing (`restrict,port-forwarding,permitlisten=…` in
# nixos-config's isis configuration); the admin key here would give root on isis
# to anyone holding this Mac's disk.
set -euo pipefail

HOST="${CONSOLE_TUNNEL_HOST:-$USER@isis.xinutec.org}"
KEY="${CONSOLE_TUNNEL_KEY:-$HOME/.ssh/console-tunnel}"
# Must match nixos-config's network.nix.
LISTEN="${CONSOLE_TUNNEL_LISTEN:-10.100.0.2:8097}"
LOCAL="${CONSOLE_TUNNEL_LOCAL:-127.0.0.1:8097}"

[ -r "$KEY" ] || { echo "no tunnel key at $KEY — see docs/agent-console.md" >&2; exit 1; }

# ssh runs in the background with a trap that kills it: killed in the foreground,
# an orphaned ssh kept the listener on isis and every redial failed to bind.
cleanup() {
  if [ -n "${SSH_PID:-}" ]; then
    kill "$SSH_PID" 2>/dev/null || true
  fi
  exit 0
}
trap cleanup EXIT INT TERM

# Redialled: sleep, a new address or an sshd restart all end the tunnel. Each
# attempt prints, so a real failure repeats rather than going quiet.
while true; do
  echo "console tunnel: ${LISTEN} → ${LOCAL} via ${HOST}"
  ssh -N \
    -i "$KEY" \
    -o IdentitiesOnly=yes \
    -o ExitOnForwardFailure=yes \
    -o ServerAliveInterval=30 \
    -o ServerAliveCountMax=3 \
    -R "${LISTEN}:${LOCAL}" \
    "$HOST" &
  SSH_PID=$!
  wait "$SSH_PID" || echo "console tunnel: dropped ($?) — redialling in ${RETRY:=10}s"
  sleep "${RETRY:-10}"
done

# ExitOnForwardFailure: an ssh that connects but cannot bind would otherwise stay
# up looking healthy. ServerAliveInterval: a half-open connection keeps isis's
# port held; 90 s of silence and this end gives up first.
