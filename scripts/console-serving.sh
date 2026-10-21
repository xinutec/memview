#!/usr/bin/env bash
# What is the running console actually serving, and is it this commit?
#
#   ./scripts/console-serving.sh
#
# The phone loads whatever the console serves on its next reload, and nothing
# says when that changes. This asks the console over HTTP rather than reading
# console-live: `STATIC_DIR` is fixed at startup and kept across SIGUSR2
# upgrades, so a console can serve another path entirely. The served JavaScript
# names its commit (scripts/stamp-version.mjs writes it into build-info.ts).
set -euo pipefail
cd "$(dirname "$0")/.."

DESK="${CONSOLE_DESK_ADDR:-127.0.0.1:8096}"
LIVE="frontend/dist/console-live"

# No --retry: it reports the last attempt and hides that nothing listened.
fetch() { curl -sf --max-time 10 "http://$DESK/$1"; }

if ! index="$(fetch index.html)"; then
  echo "no console is answering on $DESK — nothing is being served" >&2
  exit 1
fi

entry="$(printf '%s' "$index" | grep -o 'main-[A-Za-z0-9]*\.js' | head -1)"
[ -n "$entry" ] || { echo "the served index.html names no main bundle" >&2; exit 1; }

# sha:"abc1234+": a trailing + marks a dirty tree.
stamp="$(fetch "$entry" | grep -o 'sha:"[^"]*",builtAt:"[^"]*"' | head -1)"
served_sha="$(printf '%s' "$stamp" | sed -n 's/.*sha:"\([^"]*\)".*/\1/p')"
built_at="$(printf '%s' "$stamp" | sed -n 's/.*builtAt:"\([^"]*\)".*/\1/p')"
# Against the last frontend commit, not HEAD: most commits cannot change the
# bundle, and a check that says stale after each of them goes unread. Specs and
# Playwright files are not in the bundle.
frontend_sha="$(git log -1 --format=%h -- frontend/ \
  ':(exclude,glob)frontend/**/e2e/**' \
  ':(exclude,glob)frontend/**/playwright.config.*' \
  ':(exclude,glob)frontend/**/*.spec.ts')"

echo "serving   $entry"
echo "built     ${served_sha:-unknown} at ${built_at:-unknown}"
echo "frontend  $frontend_sha (last commit touching frontend/)"

ok=0
clean_sha="${served_sha%+}"

if [ -z "$served_sha" ]; then
  echo "MISMATCH  the bundle carries no build stamp — built before stamping existed" >&2
  ok=1
elif [ "$clean_sha" != "$served_sha" ]; then
  echo "DIRTY     built from $clean_sha with uncommitted changes — it is no commit" >&2
  ok=1
elif ! git cat-file -e "$clean_sha^{commit}" 2>/dev/null; then
  echo "UNKNOWN   $clean_sha is not a commit in this repo — rebased away, or another tree" >&2
  ok=1
elif git merge-base --is-ancestor "$frontend_sha" "$clean_sha"; then
  echo "ok        the served bundle has every frontend change up to $frontend_sha"
else
  echo "BEHIND    built at $clean_sha, before $frontend_sha — pnpm run publish:console" >&2
  ok=1
fi

# A different bundle on disk means this console started with another STATIC_DIR.
if [ -r "$LIVE/index.html" ]; then
  on_disk="$(grep -o 'main-[A-Za-z0-9]*\.js' "$LIVE/index.html" | head -1)"
  if [ "$on_disk" != "$entry" ]; then
    echo "ELSEWHERE $LIVE holds $on_disk — this console is serving some other path" >&2
    ok=1
  fi
fi

exit "$ok"
