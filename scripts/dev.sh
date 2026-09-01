#!/usr/bin/env bash
# Dev server on the Mac: serve the live memory corpus + built SPA, open on
# the LAN (no auth — SESSION_SECRET unset). View at http://192.168.1.81:8091
# (the Mac is headless; localhost is useless from other machines).
#
# ⚠ **The mined artefacts live under `cache/`, and pointing at the wrong path
# costs you nothing visible.** `AGENTS_FILE` here named
# `~/.claude/memview/agents.json` — one directory above where the miner has
# written it since the cache subdirectory appeared — so the dev server served a
# graph with NO usage and NO affinities. Nothing failed: absence is a designed
# normal state (`usage_of`: "a machine with no transcripts has no artefact, and
# search must still work there"), which is right for CI and wrong here, because
# it means a session judging the view locally judges a version of it with the
# interesting half missing. The canonical location is `reader::home::cache`,
# i.e. `$HOME/.claude/memview/cache/`. Found 2026-09-01 while opening the graph
# view to look at it (memview#1306).
set -euo pipefail
cd "$(dirname "$0")/.."

MEMORY_DIR="${MEMORY_DIR:-$HOME/.claude/projects/-Users-pippijn-Code/memory}"
COUSE_FILE="${COUSE_FILE:-$HOME/.claude/projects/-Users-pippijn-Code/couse.json}"
AGENTS_FILE="${AGENTS_FILE:-$HOME/.claude/memview/cache/agents.json}"
STATIC_DIR="${STATIC_DIR:-frontend/dist/memview-web/browser}"

# Say which of them are not there. A dev server that quietly serves half a graph
# teaches you the view is worse than it is.
for f in "$COUSE_FILE" "$AGENTS_FILE"; do
  [ -r "$f" ] || echo "⚠ absent, so the graph loses what it feeds: $f" >&2
done

MEMORY_DIR="$MEMORY_DIR" COUSE_FILE="$COUSE_FILE" AGENTS_FILE="$AGENTS_FILE" \
  STATIC_DIR="$STATIC_DIR" exec nix develop -c cargo run
