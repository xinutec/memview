#!/usr/bin/env bash
# Bring the mined artefacts up to date by reading only what grew.
#
#   ./scripts/mine-resume.sh
#
# ⚠ **Why this runs often rather than nightly.** Measured 2026-08-30: a resumed
# mine is ~6s where a full one is ~4m30, and reader cost is almost entirely
# CATCH-UP — `memory-rank` is 0.25s right after a mine and 2.07s after 18 MB of
# drift, the same code. Readers never write, so every one of them re-reads the
# whole day's growth until the next mine. Running this hourly holds that near
# zero for about two minutes of CPU across a day (memview#1240).
#
# ⚠ **The FULL mine stays**, in claude-sync.sh's nightly. A resumed run is only
# ever as correct as the chain of resumes behind it; the from-scratch rebuild is
# what repairs drift and what every parity check is measured against.
set -euo pipefail

# ⚠ **A launchd agent gets a bare environment with NO nix on PATH.** Without
# this the run dies with `nix: command not found` — which it did, and only
# surfaced after the lock bug was fixed, because a leaked lock made every pass
# skip before it ever reached the nix line. `claude-sync.sh` sources the same
# file for the same reason; this is not a new trick, it is the one already in use.
#
# Unconditional and non-fatal: an interactive run already has nix, and a missing
# profile should fail at the `nix` call with a real message rather than here.
# shellcheck disable=SC1091
. /nix/var/nix/profiles/default/etc/profile.d/nix-daemon.sh 2>/dev/null || true

MEMVIEW="${MEMVIEW_REPO:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"

# ⚠ **One miner at a time, and this is not belt-and-braces.** Artefacts are
# written atomically, but the PAIR — the artefacts and the resume marks — is not:
# the marks assert "the corpus up to here is already in those files". Two miners
# interleaving can leave one run's artefacts beside the other's marks, which
# promises work no artefact holds, and the next resume then skips it silently.
#
# `mkdir` is the atomic primitive that exists everywhere; macOS has no `flock`.
LOCK="${TMPDIR:-/tmp}/memview-mine.lock"

# ⚠ **A lock nobody holds must be RECLAIMABLE.** The holder's pid goes inside it,
# so a mine killed before its trap ran — or one that died with the machine — does
# not disable every later pass forever. Checked with `kill -0`, never by name:
# a `pgrep` for this script matches the check itself
# (`feedback_my_tooling_fails_silently_not_loudly`).
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
  # ⚠ **Not an error.** The nightly full mine takes ~4m30 and this fires hourly;
  # overlapping is expected and skipping is the correct response. Saying so on
  # stdout keeps a skip from reading as a run that found nothing to do.
  echo "a mine is already running (pid $(cat "$LOCK/pid" 2>/dev/null)) — skipping this pass"
  exit 0
fi
# ⚠ Removed on EVERY exit, including a failure.
trap 'rm -rf "$LOCK" 2>/dev/null || true' EXIT

cd "$MEMVIEW"
# ⚠ **NOT `exec`, and that was a real bug.** `exec` replaces this shell, so the
# EXIT trap above never fires and the lock leaks on every SUCCESSFUL run — the
# first pass then disables every pass after it. Caught by kickstarting the agent
# and reading its log rather than trusting the switch's exit code.
nix develop --no-warn-dirty -c cargo run --release --quiet --bin agents -- --resume
