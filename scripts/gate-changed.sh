#!/usr/bin/env bash
# Run the gate checks that this working tree's changes could break, with the
# GATE'S OWN argv rather than a retyped approximation.
#
#   ./scripts/gate-changed.sh          # against HEAD
#   ./scripts/gate-changed.sh --all    # every check, i.e. the full gate
#
# ⚠ **THIS IS NOT THE GATE AND MUST NEVER READ AS IT.** A subset that passes is
# not the gate passing, so it prints what it SKIPPED and says so at the end. The
# pre-commit hook is the only thing that judges a commit, and a skipped check is
# not a passed one — dev-lint's DL-NO-SILENT-CAPS.
#
# ⚠ **Not fast by default.** It costs whatever the selected checks cost, and a
# corpus or transcript change selects the slow ones. Read the SKIPPED list, not a
# remembered duration.
#
# ⚠ **The argv comes out of `gate.json` and is never written here.** A retyped
# command drifts from the one that will judge the commit, and the drift is
# invisible because the weaker command still exits 0: `cargo clippy
# --all-targets` without `--workspace` lints the root package alone and passes.
#
# ⚠ **`-e` is safe even though this must survive a failing check to collect the
# rest**: every check runs inside `if out=$(...)`, a tested command, where `set
# -e` does not fire. Dropping it is refused by DL-SHELL-STRICT-MODE.
set -euo pipefail
# An unchecked `cd` would run every line below in the wrong directory and say
# nothing — DL-SHELL-CD-UNCHECKED.
cd "$(git rev-parse --show-toplevel)" || exit 1

want_all=false
only=""
case "${1:-}" in
  --all) want_all=true ;;
  # ⚠ **A NAME runs that check with the gate's argv.** This exists so there is
  # never a reason to type `cargo clippy` by hand during a loop: the whole defect
  # this script was written for is that a retyped command drifts — `--workspace`
  # missing lints one crate of four, and still exits 0. If the right thing is not
  # addressable, the wrong thing gets typed.
  "") ;;
  *) only="$1" ;;
esac

changed=$(git status --porcelain | awk '{print $NF}')
[[ -n $changed ]] || { echo "nothing changed — the gate has nothing to narrow to"; exit 0; }

# What each check's verdict can depend on. ⚠ A check absent from this table runs
# ALWAYS: an unmapped check must be conservative, because guessing it is
# irrelevant is how a subset silently stops covering something.
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
# ⚠ Named, not counted. "skipped 11" invites reading it as covered.
for s in "${skipped[@]}"; do echo "  skipped: $s"; done
if ((${#failed[@]})); then exit 1; fi
echo "⚠ a subset passed. THIS IS NOT THE GATE — the pre-commit hook is."
