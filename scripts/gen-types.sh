#!/usr/bin/env bash
# Generate the console's TypeScript wire types from the Rust definitions.
#
#   nix develop --command scripts/gen-types.sh            # regenerate + install
#   nix develop --command scripts/gen-types.sh --check    # report drift, write nothing
#
# `--features ts` turns ts-rs on; normal builds carry none of it. The export
# tests are named export_bindings_*, so the filter runs generation and nothing
# else. Large integers become `number`: the wire is JSON, where they already are.
#
# Generating into scratch, installing only on success and comparing by content
# is dev-lint's gen-types, shared with the other repositories that do this.
set -euo pipefail
cd "$(dirname "$0")/.."

export TS_RS_LARGE_INT=number
exec nix run "git+file:../dev-lint?ref=HEAD#gen-types" -- "$@" \
  --out frontend/projects/console-web/src/app/generated \
  -- cargo test -p console -p reader --lib --features console/ts export_bindings
