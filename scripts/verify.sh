#!/usr/bin/env bash
# memview verify — Rust backend (fmt + clippy + tests) + Angular frontend (lint
# + unit tests + build + phone-width layout harness) + the shared dev-lint
# rules. Toolchain comes from the flake devshell (rev-pinned via flake.lock),
# so it's reproducible without cargo/npm on PATH.
set -euo pipefail
cd "$(dirname "$0")/.."

nix develop -c bash -c '
  set -euo pipefail
  # @angular/build:application tears down its Piscina worker pool at process
  # exit; on macOS / Node 24 / libuv 1.52 that teardown intermittently aborts
  # the process (a libuv kqueue assertion, "errno == EINTR") AFTER the bundle
  # is complete. NG_BUILD_MAX_WORKERS=1 lowers the rate but does not eliminate
  # it, so the dist check below is authoritative over the exit status.
  export NG_BUILD_MAX_WORKERS=1

  cargo fmt --all --check
  cargo clippy --all-targets -- -D warnings
  cargo test

  cd frontend
  # `--frozen-lockfile` is pnpm ci: install exactly pnpm-lock.yaml, or fail. The
  # guard is not just a speed-up — without it a node_modules left behind by npm
  # still has a working .bin, so verify would pass against packages the lockfile
  # no longer describes.
  if [ ! -x node_modules/.bin/eslint ] || [ pnpm-lock.yaml -nt node_modules ]; then
    pnpm install --frozen-lockfile
  fi
  pnpm run lint
  pnpm test
  pnpm run build || true
  # Authoritative build check: an empty/missing index.html means the bundle
  # really failed; a nonzero exit with a good bundle was the kqueue flake.
  test -s dist/memview-web/browser/index.html
  pnpm run ui-check

  cd ..
  # The graph layout, measured rather than looked at. Every bug this view has had
  # was a picture that looked plausible — a zoom that was a silent no-op, labels
  # that overprinted, sections that smeared into one ball — and none of them
  # threw or failed a lint. These thresholds are the only gate that can catch the
  # next one before it ships.
  node scripts/graph-report.mjs
'

# The corpus itself, when it is present. Guarded rather than required: CI has no
# memory directory (the image ships the viewer, never the memories), and a gate
# that cannot run there would either fail every build or be quietly skipped in
# both places. Locally it is the check that matters most — the app can be
# perfect and the document set still be falling apart.
MEMORY_DIR="${MEMORY_DIR:-$HOME/.claude/projects/-Users-pippijn-Code/memory}"
if [[ -f $MEMORY_DIR/MEMORY.md ]]; then
  nix develop -c cargo run --quiet --bin memory-lint "$MEMORY_DIR" | tail -20
  # The co-use artefact is NOT regenerated here: it reads every byte of every
  # session transcript (~3 GB, about two minutes). memory-lint uses whatever is
  # already on disk, so this gate stays fast and the mining is run deliberately:
  #     nix develop -c cargo run --release --bin couse
else
  echo "no corpus at $MEMORY_DIR — skipping memory-lint"
fi

# Shared fleet rules over the whole repo (nix run, never result/bin — a pinned
# build goes stale and silently misses rules shipped since).
nix run "$HOME/Code/dev-lint" -- .

echo "verify OK"
