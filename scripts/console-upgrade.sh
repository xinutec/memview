#!/usr/bin/env bash
# Build the console and move a running one onto it, without losing a session.
#
#   ./scripts/console-upgrade.sh          # build, install, upgrade in place
#   ./scripts/console-upgrade.sh --install-only
#
# **Why this exists rather than `launchctl kickstart -k`.** kickstart sends
# SIGTERM. The console answers SIGTERM by shutting every session down on purpose
# — an orphaned `claude` keeps its id and its process-table row, where
# `past::in_use` then refuses to resume the very conversation nobody is using. So
# the obvious restart is the one that costs every open conversation, and this
# script exists so the right way is also the easy way.
#
# SIGUSR2 is the upgrade signal, spelled the way nginx spells it and for the same
# reason: `kill` means stop, and an upgrade answering to it would be a stop that
# sometimes did not stop. `Roster::handover` catches it and `execve`s the binary
# — **same pid**, so the children stay children, their stdin/stdout/stderr are
# carried across in a HANDOVER env var, and launchd never sees a restart at all.
#
# ⚠ **Install by atomic rename.** macOS refuses to write to a running executable,
# but replacing the directory entry is fine: the running process keeps its inode
# and the path gains the new build, which is exactly what the re-exec then picks
# up. Writing in place would fail; deleting first would leave a window where the
# service cannot start.
#
# ⚠ **If the handover fails the console keeps running the OLD build**, holding
# everything it held. That is deliberate in the runner — exiting on a failed exec
# would leave live `claude` processes with nobody holding their stdin — so a
# failure here is a no-op, not an outage.
set -euo pipefail
cd "$(dirname "$0")/.."

CONSOLE_BIN="${CONSOLE_BIN:-$HOME/.local/libexec/agent-console}"
DESK="${CONSOLE_DESK_ADDR:-127.0.0.1:8096}"

echo "building..."
nix develop -c cargo build -p console --release
built="$(nix develop -c cargo metadata --format-version 1 --no-deps \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/release/console"
[ -x "$built" ] || { echo "no binary at $built" >&2; exit 1; }

mkdir -p "$(dirname "$CONSOLE_BIN")"
# Same directory, so the rename cannot cross a filesystem and stays atomic.
staged="$CONSOLE_BIN.incoming.$$"
cp "$built" "$staged"
chmod +x "$staged"
mv -f "$staged" "$CONSOLE_BIN"
echo "installed -> $CONSOLE_BIN"

# An `if`, not `[ … ] && exit 0`: under `set -e` an AND-list whose test fails is
# itself a failed statement, so the common path — no argument at all — would have
# exited 0 here without ever upgrading anything.
if [ "${1:-}" = "--install-only" ]; then
  exit 0
fi

# By pid, and the pid comes from whoever is LISTENING on the desk port — never by
# name. `pgrep console` would also match this script, an editor, and anything else
# with the word in its command line; the port has exactly one owner.
listening() {
  lsof -nP -iTCP:"${DESK##*:}" -sTCP:LISTEN -Fp 2>/dev/null | sed -n 's/^p//p' | head -1
}
before="$(listening)"
if [ -z "$before" ]; then
  echo "no console is running — launchctl kickstart org.xinutec.agent-console to start one"
  exit 0
fi

echo "upgrading pid $before in place (SIGUSR2)"
kill -USR2 "$before"

# The re-exec keeps the pid, so "did it work" is not "is it alive" — it is
# "is the SAME pid serving again". A dead pid means the handover exited, which is
# the one outcome the runner is written to avoid.
for _ in $(seq 1 30); do
  if curl -sf -o /dev/null "http://$DESK/api/state"; then
    after="$(listening)"
    if [ "$after" = "$before" ]; then
      echo "upgraded: pid $after, sessions kept"
      exit 0
    fi
    echo "WARNING: serving again but pid changed $before -> $after; sessions were NOT carried" >&2
    exit 1
  fi
  sleep 1
done

echo "the desk did not answer within 30s — check ~/Library/Logs/agent-console.log" >&2
exit 1
