#!/usr/bin/env bash
# What is the running console actually serving, and is it this commit?
#
#   ./scripts/console-serving.sh
#
# **The question this exists to answer.** `frontend/dist/console-live` is a plain
# directory that any build could once write into, and the phone loads whatever is
# in it on its next reload. Nothing announced a change and nothing recorded one,
# so "is the fix I am testing actually the code being served?" had no answer
# short of reading a hash by hand — which is how an ablated build (memview#116's
# fix DELETED) came to be live for eight minutes on 2026-08-11 with neither of us
# told. `build:console` no longer publishes; `publish:console` does. This is the
# other half: a way to ask.
#
# ⚠ **It asks the console over HTTP, not the directory.** `STATIC_DIR` is read
# from the environment at startup and `SIGUSR2` re-execs that same environment,
# so a console can go on serving a path nobody would think to look at — it has
# happened, for three upgrades in a row (docs/agent-console.md, "ng build deletes
# its whole output path"). Reading console-live would have agreed with itself and
# been wrong. The desk's own answer is the only one that is about what the phone
# gets.
#
# The bundle carries its own provenance: scripts/stamp-version.mjs embeds the git
# sha and build time into build-info.ts before every build, so the served
# JavaScript says which commit it came from. That is what is read back here.
set -euo pipefail
cd "$(dirname "$0")/.."

DESK="${CONSOLE_DESK_ADDR:-127.0.0.1:8096}"
LIVE="frontend/dist/console-live"

# No --retry: a retry on a refused connection reports the LAST attempt's failure
# and hides that nothing was ever listening.
fetch() { curl -sf --max-time 10 "http://$DESK/$1"; }

if ! index="$(fetch index.html)"; then
  echo "no console is answering on $DESK — nothing is being served" >&2
  exit 1
fi

entry="$(printf '%s' "$index" | grep -o 'main-[A-Za-z0-9]*\.js' | head -1)"
[ -n "$entry" ] || { echo "the served index.html names no main bundle" >&2; exit 1; }

# sha:"abc1234+" — the trailing + is stamp-version.mjs marking a dirty tree.
stamp="$(fetch "$entry" | grep -o 'sha:"[^"]*",builtAt:"[^"]*"' | head -1)"
served_sha="$(printf '%s' "$stamp" | sed -n 's/.*sha:"\([^"]*\)".*/\1/p')"
built_at="$(printf '%s' "$stamp" | sed -n 's/.*builtAt:"\([^"]*\)".*/\1/p')"
head_sha="$(git rev-parse --short HEAD)"

echo "serving   $entry"
echo "built     ${served_sha:-unknown} at ${built_at:-unknown}"
echo "HEAD      $head_sha"

ok=0

if [ -z "$served_sha" ]; then
  echo "MISMATCH  the bundle carries no build stamp — built before stamping existed" >&2
  ok=1
elif [ "$served_sha" = "$head_sha" ]; then
  echo "ok        the served bundle was built from HEAD"
elif [ "$served_sha" = "$head_sha+" ]; then
  echo "DIRTY     built from HEAD with uncommitted changes — it is not any commit" >&2
  ok=1
else
  echo "BEHIND    the served bundle is ${served_sha}, not $head_sha — publish:console" >&2
  ok=1
fi

# The directory the phone is meant to be fed from. Disagreeing with what the desk
# hands out means this console started with a different STATIC_DIR and kept it.
if [ -r "$LIVE/index.html" ]; then
  on_disk="$(grep -o 'main-[A-Za-z0-9]*\.js' "$LIVE/index.html" | head -1)"
  if [ "$on_disk" != "$entry" ]; then
    echo "ELSEWHERE $LIVE holds $on_disk — this console is serving some other path" >&2
    ok=1
  fi
fi

exit "$ok"
