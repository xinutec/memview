#!/usr/bin/env bash
# Bring the mined artefacts up to date by reading only what grew.
#
#   ./scripts/mine-resume.sh
#
# Hourly, not nightly: readers never write, so each re-reads the growth since the
# last mine, and a resumed mine costs seconds where a full one costs minutes.
# The full mine stays in claude-sync.sh's nightly: it repairs any drift a chain
# of resumes builds up, and parity checks compare against it.
set -euo pipefail

# launchd's bare environment has no nix on PATH. Non-fatal: an interactive shell
# has nix already.
# shellcheck disable=SC1091 # the nix profile lives where the machine put it
. /nix/var/nix/profiles/default/etc/profile.d/nix-daemon.sh 2>/dev/null || true

MEMVIEW="${MEMVIEW_REPO:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"

# One miner at a time: the artefacts and the resume marks are written separately,
# and two runs interleaving could leave marks promising work no artefact holds.
# `mkdir`, as macOS has no `flock`.
LOCK="${TMPDIR:-/tmp}/memview-mine.lock"

# The holder's pid is inside, so a lock left by a dead run is reclaimed; checked
# with `kill -0`, as a `pgrep` for this script matches the check itself.
take_lock() {
  if mkdir "$LOCK" 2>/dev/null; then
    echo $$ > "$LOCK/pid"
    return 0
  fi
  local held
  held=$(cat "$LOCK/pid" 2>/dev/null || echo "")
  if [[ -n $held ]] && kill -0 "$held" 2>/dev/null; then
    return 1 # genuinely held by a live process
  fi
  echo "⚠ reclaiming a lock left by pid ${held:-unknown}, which is gone" >&2
  rm -rf "$LOCK"
  mkdir "$LOCK" 2>/dev/null && echo $$ > "$LOCK/pid"
}

if ! take_lock; then
  # Overlap with the nightly is expected; say so, so a skip does not read as a
  # run that found nothing.
  echo "a mine is already running (pid $(cat "$LOCK/pid" 2>/dev/null)) — skipping this pass"
  exit 0
fi
trap 'rm -rf "$LOCK" 2>/dev/null || true' EXIT

cd "$MEMVIEW"
# Not `exec`: it would skip the EXIT trap and leak the lock on every success.
# `--exports none`: the console's files are left to the nightly full mine, as a
# resumed run would build a timeline of the tail alone.
nix develop --no-warn-dirty -c cargo run --release --quiet --bin agents -- --resume --exports none
