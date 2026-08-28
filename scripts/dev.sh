#!/usr/bin/env bash
# Dev server on the Mac: serve the live memory corpus + built SPA, open on
# the LAN (no auth — SESSION_SECRET unset). View at http://192.168.1.81:8091
# (the Mac is headless; localhost is useless from other machines).
set -euo pipefail
cd "$(dirname "$0")/.."

MEMORY_DIR="${MEMORY_DIR:-$HOME/.claude/projects/-Users-pippijn-Code/memory}" \
  COUSE_FILE="${COUSE_FILE:-$HOME/.claude/projects/-Users-pippijn-Code/couse.json}" \
  AGENTS_FILE="${AGENTS_FILE:-$HOME/.claude/memview/agents.json}" \
  STATIC_DIR="${STATIC_DIR:-frontend/dist/memview-web/browser}" \
  exec nix develop -c cargo run
