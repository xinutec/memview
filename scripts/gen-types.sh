#!/usr/bin/env bash
# Generate the console's TypeScript wire types from the Rust definitions.
#
#   nix develop --command scripts/gen-types.sh            # regenerate + install
#   nix develop --command scripts/gen-types.sh --check    # report drift, write nothing
#
# `--features ts` enables ts-rs; the `export_bindings` filter runs generation
# only. Large integers become `number`, as they already are in JSON. dev-lint's
# gen-types generates into scratch and installs only on success.
set -euo pipefail
cd "$(dirname "$0")/.."

export TS_RS_LARGE_INT=number
exec nix run "git+file:../dev-lint?ref=HEAD#gen-types" -- "$@" \
  --out frontend/projects/console-web/src/app/generated \
  -- cargo test -p console -p reader --lib --features console/ts export_bindings
