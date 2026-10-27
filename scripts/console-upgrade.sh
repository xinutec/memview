#!/usr/bin/env bash
# Build the console and move a running one onto it, without losing a session.
#
#   ./scripts/console-upgrade.sh          # build, install, upgrade in place
#   ./scripts/console-upgrade.sh --install-only
#
# Not `launchctl kickstart -k`: its SIGTERM is the console's stop path, which
# ends every session (an orphaned `claude` would block resuming its own
# conversation). SIGUSR2, as in nginx, makes `Roster::handover` `execve` the new
# binary in the same pid: the children and their pipes survive and launchd sees
# no restart. A failed handover leaves the old build running, holding everything.
#
# Installed by atomic rename: macOS refuses writes to a running executable, and
# the running process keeps its inode.
set -euo pipefail
cd "$(dirname "$0")/.."

CONSOLE_BIN="${CONSOLE_BIN:-$HOME/.local/libexec/agent-console}"
DESK="${CONSOLE_DESK_ADDR:-127.0.0.1:8096}"

# The flake build cannot see an untracked file; a new module nothing references
# yet would be left out silently.
untracked="$(git ls-files --others --exclude-standard -- console reader src)"
if [ -n "$untracked" ]; then
  echo "untracked source — nix will not see these, git add them first:" >&2
  printf '  %s\n' $untracked >&2
  exit 1
fi

# Built by nix, not the dev shell's cargo, so the install is reproducible from the
# commit. It is slower (the whole vendored tree recompiles); iterate with
# `cargo run -p console`.
LIBEXEC="$(dirname "$CONSOLE_BIN")"
mkdir -p "$LIBEXEC"

# The out-link is a GC root: the copy loads libiconv from the store, and a
# collected path would stop it at its next start. It also names the build's source.
echo "building..."
out="$(nix build .#console --out-link "$LIBEXEC/.agent-console-build" --print-out-paths)"
built="$out/bin/console"
[ -x "$built" ] || { echo "no binary at $built" >&2; exit 1; }

# Staged in the destination directory so the rename stays on one filesystem.
install_as() {
  local staged="$2.incoming.$$"
  cp "$1" "$staged"
  chmod +x "$staged"
  mv -f "$staged" "$2"
  echo "installed -> $2"
}

install_as "$built" "$CONSOLE_BIN"

# The launchd job runs this installed copy: under launchd, Apple's binaries are
# refused the external volume the checkout lives on (see console-service.sh).
install_as scripts/console-service.sh "$LIBEXEC/agent-console-service"

if [ "${1:-}" = "--install-only" ]; then
  exit 0
fi

# The pid listening on the desk port; `pgrep console` matches this script too.
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

# Success is the SAME pid serving again; a new pid means the sessions were lost.
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
