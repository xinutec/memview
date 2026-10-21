#!/usr/bin/env bash
# Every workspace member is named in three places. Check, do not remember.
#
#   ./scripts/workspace-members.sh
#
# Cargo loads every member's manifest first, so a member missing from a build's
# source fails that build. None of these lists is generated:
#
#   Cargo.toml   `members` — the truth
#   Dockerfile   a `COPY <m>/Cargo.toml` and a stub in the dep-caching layer
#   flake.nix    the `fileset` the console package is built from
#
# The gate does not build the image, so without this a new member turns the image
# job red only on push.
set -euo pipefail

cd "$(dirname "$0")/.."

members=$(sed -n 's/^members = \[\(.*\)\]$/\1/p' Cargo.toml | tr -d '" ' | tr ',' '\n')
if [[ -z $members ]]; then
  echo "could not read \`members\` from Cargo.toml — has the format changed?" >&2
  exit 2
fi

missing=0
for member in $members; do
  # The image needs the manifest, and a stub directory to put it in.
  grep -qF "COPY $member/Cargo.toml" Dockerfile ||
    { echo "✗ Dockerfile: no \`COPY $member/Cargo.toml\`"; missing=1; }
  grep -qE "mkdir -p .*\b$member/src\b" Dockerfile ||
    { echo "✗ Dockerfile: $member/src is not in the stub \`mkdir -p\`"; missing=1; }
  grep -qE "^\s+echo '' > $member/src/lib\.rs|$member/src/lib\.rs" Dockerfile ||
    { echo "✗ Dockerfile: nothing writes $member/src/lib.rs"; missing=1; }
  # The console package is built from an explicit fileset, not the git tree.
  grep -qE "^\s+\./$member$" flake.nix ||
    { echo "✗ flake.nix: ./$member is not in the console package's fileset"; missing=1; }
done

if (( missing )); then
  echo
  echo "A workspace member is missing from a build that has to load its manifest." >&2
  echo "Add it, or the image job goes red on push while this gate stays green." >&2
  exit 1
fi

echo "workspace members consistent across Cargo.toml, Dockerfile and flake.nix:"
printf '  %s\n' $members
