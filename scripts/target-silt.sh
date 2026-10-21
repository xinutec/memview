#!/usr/bin/env bash
# How many files the build tree holds, failing past the threshold.
#
#   ./scripts/target-silt.sh            # report
#   ./scripts/target-silt.sh --prune    # move the silted directories aside
#
# Cargo never prunes stale artefacts and exec cost grows with the files in the
# directory, so a tree gated on every commit slows its own fresh binaries.
#
# Moved aside, not deleted: `mv` is atomic, and deleting tens of gigabytes under
# a running build is not.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

# Past this the exec tax costs more than a rebuild.
LIMIT=${TARGET_SILT_LIMIT:-100000}

silted=()
for deps in target/*/deps; do
    # A glob that matched nothing, or a stale copy already moved aside.
    [[ -d $deps ]] || continue
    [[ $deps == target/*.stale-*/deps ]] && continue
    # `ls -f` neither sorts nor stats, so this stays fast at any size.
    count=$(ls -f "$deps" | wc -l | tr -d ' ')
    printf '%-28s %10s files\n' "$deps" "$count"
    if (( count > LIMIT )); then
        silted+=("$(dirname "$deps")")
    fi
done

# Names only: sizing them takes seconds on every commit (`du -sh target/*.stale-*`).
stale=(target/*.stale-*)
if [[ -d ${stale[0]:-} ]]; then
    echo
    echo "Stale copies from earlier prunes, kept rather than deleted:"
    printf '  %s\n' "${stale[@]}"
fi

if (( ${#silted[@]} == 0 )); then
    exit 0
fi

if [[ ${1:-} == --prune ]]; then
    for dir in "${silted[@]}"; do
        to="$dir.stale-$(date +%F)"
        # A second prune the same day would move the tree INTO the first copy.
        [[ -e $to ]] && to="$dir.stale-$(date +%F-%H%M%S)"
        mv "$dir" "$to"
        echo "moved $dir aside; cargo will rebuild"
    done
    exit 0
fi

echo
echo "Past $LIMIT files, where the exec tax is worth a rebuild. Run:"
echo "    ./scripts/target-silt.sh --prune"
exit 1
