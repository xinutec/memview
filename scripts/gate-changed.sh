#!/usr/bin/env bash
# Run the gate checks that this working tree's changes could break — with the
# GATE'S OWN argv, never a retyped approximation.
#
#   ./scripts/gate-changed.sh          # against HEAD
#   ./scripts/gate-changed.sh --all    # every check, i.e. the full gate
#
# ⚠ **THIS IS NOT THE GATE AND MUST NEVER READ AS IT.** A subset that passes is
# not the gate passing. It prints what it SKIPPED and says so at the end, because
# a silent subset is exactly the failure it exists to prevent — the pre-commit
# hook remains the only thing that judges a commit.
#
# ⚠ **Why it exists at all.** Measured 2026-08-29: the full gate is ~13 minutes,
# so during an edit loop it is not run, and what gets run instead is fast and
# WRONG. That day I typed `cargo clippy --all-targets` after nearly every edit
# and reported "clippy exit=0" each time — the gate runs `--workspace
# --all-targets`, and without it cargo lints the ROOT PACKAGE ONLY. One crate of
# four. The same day, `cargo test` built 39 test targets where `--workspace`
# builds 94. Both true, both green, both 42% of the claim.
#
# ⚠ **So the argv comes out of `gate.json` and is not written here.** That is the
# whole mechanism: a hand-typed command drifts from the one that will judge the
# commit, and the drift is invisible because the weaker command still exits 0.
#
# ⚠ **A skipped check is not a passed check** — see `dev-lint`'s own rule about
# reporting what was not run (DL-NO-SILENT-CAPS).
# ⚠ **`-e` is safe here even though this must survive a failing check**: every
# check runs inside `if out=$(...)`, a TESTED command, where `set -e` does not
# fire. Omitting it was the first instinct and dev-lint was right to refuse —
# DL-SHELL-STRICT-MODE.
set -euo pipefail
# ⚠ `|| exit 1` because this deliberately does NOT `set -e` — it must survive a
# failing check to collect the rest — so an unchecked `cd` would run every line
# below in the wrong directory and say nothing (DL-SHELL-CD-UNCHECKED, caught by
# dev-lint on the first run of this very script).
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
