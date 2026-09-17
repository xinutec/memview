#!/usr/bin/env bash
# The Dockerfile caches dependency compilation by copying only the manifests and
# compiling against stub sources, long before `COPY src/ src/`. A manifest that
# NAMES a target — `[[bin]] name = "agents"` — requires that target's file to
# exist at parse time, so declaring one breaks that layer while every local check
# stays green: the gate does not build the image.
#
# This rebuilds the same stub tree and asks cargo to parse it.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

# Named separately from a parse failure below: without this, an absent cargo is
# reported as "the manifests do not parse", which is a different bug entirely.
if ! command -v cargo >/dev/null; then
  echo "cargo is not on PATH; run this inside the dev shell." >&2
  exit 2
fi

stub=$(mktemp -d)
trap 'rm -rf "$stub"' EXIT

cp Cargo.toml Cargo.lock "$stub/"
for crate in console reader bash-oracle; do
  mkdir -p "$stub/$crate/src"
  cp "$crate/Cargo.toml" "$stub/$crate/"
done

# Mirrors the Dockerfile's stub layer. A `default-run` that does not exist fails
# to parse, so the two packages that name one get a main.rs.
mkdir -p "$stub/src"
echo 'fn main() {}' > "$stub/src/main.rs"
echo '' > "$stub/src/lib.rs"
echo 'fn main() {}' > "$stub/console/src/main.rs"
echo '' > "$stub/console/src/lib.rs"
echo '' > "$stub/reader/src/lib.rs"
echo '' > "$stub/bash-oracle/src/lib.rs"

if ! out=$(cd "$stub" && cargo metadata --no-deps --format-version 1 --offline 2>&1); then
  echo "The manifests do not parse without their sources, so the image build fails:"
  echo "$out" | sed 's/^/  /'
  echo "A target named in a Cargo.toml must have its file in the Dockerfile's stub layer."
  exit 1
fi
