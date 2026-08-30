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

MEMVIEW="${MEMVIEW_REPO:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"

# ⚠ **One miner at a time, and this is not belt-and-braces.** Artefacts are
# written atomically, but the PAIR — the artefacts and the resume marks — is not:
# the marks assert "the corpus up to here is already in those files". Two miners
# interleaving can leave one run's artefacts beside the other's marks, which
# promises work no artefact holds, and the next resume then skips it silently.
#
# `mkdir` is the atomic primitive that exists everywhere; macOS has no `flock`.
LOCK="${TMPDIR:-/tmp}/memview-mine.lock"
if ! mkdir "$LOCK" 2>/dev/null; then
  # ⚠ **Not an error.** The nightly full mine takes ~4m30 and this fires hourly;
  # overlapping is expected and skipping is the correct response. Saying so on
  # stdout keeps a skip from reading as a run that found nothing to do.
  echo "a mine is already running ($LOCK) — skipping this pass"
  exit 0
fi
# ⚠ Removed on EVERY exit, including a failure: a lock left behind by a crashed
# mine would silently disable every later pass, and nothing would report it.
trap 'rmdir "$LOCK" 2>/dev/null || true' EXIT

cd "$MEMVIEW"
exec nix develop --no-warn-dirty -c cargo run --release --quiet --bin agents -- --resume
