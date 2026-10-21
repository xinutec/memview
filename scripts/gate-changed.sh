#!/usr/bin/env bash
# Run the gate checks this working tree's changes could break, with the gate's
# own argv.
#
#   ./scripts/gate-changed.sh          # against HEAD
#   ./scripts/gate-changed.sh --all    # every check, i.e. the full gate
#   ./scripts/gate-changed.sh <name>   # the checks whose name contains it
#
# NOT THE GATE: a passing subset is not a passing gate, so it names what it
# skipped. The argv is read from `gate.json`, never retyped: `cargo clippy
# --all-targets` without `--workspace` lints one crate and still passes.
#
# `-e` holds although a failing check must not stop the rest: each runs inside
# `if out=$(...)`, where `set -e` does not fire.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

want_all=false
only=""
case "${1:-}" in
  --all) want_all=true ;;
  "") ;;
  *) only="$1" ;;
esac

changed=$(git status --porcelain | awk '{print $NF}')
[[ -n $changed ]] || { echo "nothing changed — the gate has nothing to narrow to"; exit 0; }

# What each check's verdict can depend on. An unmapped check always runs.
matches() {
  case "$1" in
    formatting|clippy|tests|"workspace members are in every build that loads them")
      grep -qE '\.rs$|Cargo\.(toml|lock)$' <<<"$changed" ;;
    "memory-lint (the corpus)")
      grep -qE '\.rs$|/memory/.*\.md$' <<<"$changed" ;;
    "transcript-lint (the conversations)")
      grep -qE '\.rs$' <<<"$changed" ;;
    frontend*|"the live-bundle pruner"|"graph layout report")
      grep -qE '^frontend/|\.ts$|\.html$|\.scss$|package\.json$|pnpm-lock' <<<"$changed" ;;
    "the console package builds (what this repo publishes)")
      grep -qE '^console/|\.rs$|flake\.(nix|lock)$' <<<"$changed" ;;
    "the table matches its Dhall")
      grep -qE 'gate\.(dhall|json)$' <<<"$changed" ;;
    *) return 0 ;;
  esac
}

names=$(python3 -c "import json;[print(c['name']) for c in json.load(open('gate.json'))['checks']]")
ran=0; skipped=(); failed=()
while IFS= read -r name; do
  if [[ -n $only ]]; then
    [[ $name == *"$only"* ]] || { skipped+=("$name"); continue; }
  elif ! $want_all && ! matches "$name"; then skipped+=("$name"); continue; fi
  mapfile -t argv < <(python3 -c "
import json,sys
for c in json.load(open('gate.json'))['checks']:
    if c['name']==sys.argv[1]:
        [print(a) for a in c['argv']]" "$name")
  printf '  %-52s ' "$name"
  if out=$("${argv[@]}" 2>&1); then echo "ok"; else echo "FAILED"; printf '%s\n' "$out" | tail -25; failed+=("$name"); fi
  ran=$((ran+1))
done <<<"$names"

echo
echo "ran $ran, skipped ${#skipped[@]}, failed ${#failed[@]}"
for s in "${skipped[@]}"; do echo "  skipped: $s"; done
if ((${#failed[@]})); then exit 1; fi
echo "⚠ a subset passed. THIS IS NOT THE GATE — the pre-commit hook is."
