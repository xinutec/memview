#!/bin/sh
# Full verification: backend fmt+clippy+tests, frontend lint+tests+build.
# Run from the repo root (nix develop supplies cargo + node).
set -eu

cd "$(dirname "$0")/.."

nix develop -c cargo fmt --check
nix develop -c cargo clippy --all-targets -- -D warnings
nix develop -c cargo test

cd frontend
nix develop .. -c npm run lint
nix develop .. -c npm test
# The Mac esbuild kqueue crash can clip dist/ on exit; the index check is
# authoritative — retry once if it was clipped.
nix develop .. -c npm run build || true
if [ ! -s dist/memview-web/browser/index.html ]; then
  nix develop .. -c npm run build
  test -s dist/memview-web/browser/index.html
fi

echo "verify OK"
