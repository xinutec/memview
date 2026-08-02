#!/usr/bin/env bash
# The console's way in, dialled from this side.
#
#   ./scripts/console-tunnel.sh
#
# Asks isis to listen on its VPN address and hand whatever arrives back down this
# connection, to the console on loopback. **Nothing initiates toward this Mac** —
# the one-way VPN rule stands unamended, and there is no firewall exception
# anywhere.
#
# What isis carries is ciphertext. The phone's TLS session terminates here, at the
# console's own gate, so isis holds no key that opens anything: it can drop the
# tunnel, which is denial of service and unavoidable for anything in the middle,
# and it cannot read, inject or impersonate. That is the whole reason this is a
# byte pipe and not a service that understands what it is carrying.
#
# The key is its own and is allowed to do exactly one thing: `restrict,
# port-forwarding,permitlisten="10.100.0.2:8097"` in nixos-config's isis
# configuration. An unattended tunnel on the ordinary admin key would give
# anything holding this Mac's disk a root session there.
set -euo pipefail
cd "$(dirname "$0")/.."

HOST="${CONSOLE_TUNNEL_HOST:-pippijn@isis.xinutec.org}"
KEY="${CONSOLE_TUNNEL_KEY:-$HOME/.ssh/console-tunnel}"
# isis's VPN address and the console's port. Both ends have to agree, and the
# other end is nixos-config's network.nix.
LISTEN="${CONSOLE_TUNNEL_LISTEN:-10.100.0.2:8097}"
LOCAL="${CONSOLE_TUNNEL_LOCAL:-127.0.0.1:8097}"

[ -r "$KEY" ] || { echo "no tunnel key at $KEY — see docs/agent-console.md" >&2; exit 1; }

# ⚠ Take the ssh with us. `ssh` here is a child of this script, and killing the
# script while ssh runs in the foreground orphans it: it keeps the listener on
# isis, so the next tunnel cannot bind, `ExitOnForwardFailure` does its job, and
# the console redials in a loop against its own predecessor. Observed exactly
# that — `remote port forwarding failed for listen port 8097` every ten seconds,
# with a healthy-looking `ssh` from a console stopped an hour earlier still
# holding the port. So it is backgrounded and waited on, with a trap that ends
# it.
cleanup() {
  # Guarded rather than chained with &&: under `set -e` a false test is a failed
  # command, and the trap would leave before doing the one thing it exists for.
  if [ -n "${SSH_PID:-}" ]; then
    kill "$SSH_PID" 2>/dev/null || true
  fi
  exit 0
}
trap cleanup EXIT INT TERM

# Redialled rather than run once. A tunnel is only useful while it is up, and the
# things that end it — sleep, a changed address, isis restarting sshd — are all
# ordinary. Each attempt is announced, so a tunnel that is failing for a real
# reason reads as a repeating message rather than as silence.
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

# ExitOnForwardFailure is the load-bearing flag. Without it, an ssh that connects
# but cannot bind the listener — the port still held by a dead tunnel, the key's
# permitlisten changed — stays up looking healthy while nothing can reach the
# console. Exiting instead lets whatever supervises this restart it, and makes the
# failure visible as a flapping service rather than as a phone that silently
# stopped working.
#
# ServerAliveInterval is the other half: a laptop lid, a NAT timeout or a dropped
# route leaves a half-open connection that isis still believes owns the listening
# port, so the next attempt cannot bind. Ninety seconds of silence and this end
# gives up first.
