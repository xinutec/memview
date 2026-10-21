#!/usr/bin/env bash
# Dev server on the Mac: serve the live memory corpus + built SPA, open on
# the LAN (no auth — SESSION_SECRET unset). View at http://192.168.1.81:8091
# (the Mac is headless; localhost is useless from other machines).
#
# The mined artefacts are in `$HOME/.claude/memview/cache/`; a wrong path fails
# nothing and serves a graph with no usage.
set -euo pipefail
cd "$(dirname "$0")/.."

# Claude Code names a project directory after its path, every `/` a `-`.
project="$HOME/.claude/projects/$(printf %s "$HOME/Code" | tr -c 'A-Za-z0-9-' -)"
MEMORY_DIR="${MEMORY_DIR:-$project/memory}"
COUSE_FILE="${COUSE_FILE:-$project/couse.json}"
AGENTS_FILE="${AGENTS_FILE:-$HOME/.claude/memview/cache/agents.json}"
STATIC_DIR="${STATIC_DIR:-frontend/dist/memview-web/browser}"

# Say which are missing, so half a graph is not mistaken for the view.
for f in "$COUSE_FILE" "$AGENTS_FILE"; do
  [ -r "$f" ] || echo "⚠ absent, so the graph loses what it feeds: $f" >&2
done

MEMORY_DIR="$MEMORY_DIR" COUSE_FILE="$COUSE_FILE" AGENTS_FILE="$AGENTS_FILE" \
  STATIC_DIR="$STATIC_DIR" exec nix develop -c cargo run
