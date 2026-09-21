#!/usr/bin/env bash
# Cargo never prunes stale artefacts, and exec cost is O(files in the directory).
#
#   ./scripts/target-silt.sh            # report, and fail past the threshold
#   ./scripts/target-silt.sh --prune    # move the silted directories aside
#
# ⚠ **The gate is what does this to itself.** Every commit writes fresh binaries
# and cargo keeps every older one for ever, so `target/debug/deps` went from
# 4,884 to 639,616 in THREE DAYS (2026-09-17 to 09-20). Nothing pruned it,
# because nothing was ever built to; the memory note said "check weekly", which
# is a chore nobody scheduled.
#
# ⚠ **The cost is not theoretical and it is not linear in anything you can see.**
# The tax falls on the FIRST exec of each newly written binary, and a test suite
# starts dozens. Measured on this repo at 639,616 files against a pruned tree:
#
#     rustdoc row    111.2s -> 44.2s
#     tests row       34.0s -> 20.0s
#     whole gate       6.8  -> 5.9 minutes
#
# Moved, never deleted: `mv` is atomic and cargo simply rebuilds, where a delete
# of 35 GB mid-build is neither. What to do with the stale copies afterwards is a
# decision for a person, which is why this only ever reports them.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

# Past this, the exec tax is worth a rebuild. The memory note's figure, and the
# measurements above are what it is drawn from.
LIMIT=${TARGET_SILT_LIMIT:-100000}

silted=()
for deps in target/*/deps; do
    # A glob that matched nothing, or a stale copy already moved aside.
    [[ -d $deps ]] || continue
    [[ $deps == target/*.stale-*/deps ]] && continue
    # `ls -f` does not sort or stat, which is what keeps this ~2s at 640k files.
    count=$(ls -f "$deps" | wc -l | tr -d ' ')
    printf '%-28s %10s files\n' "$deps" "$count"
    # An `if`, not `(( … )) && …`: a false arithmetic test returns 1, and under
    # `set -e` that is exempt only because it is not the final command of the
    # AND-list. One edit away from killing the script silently.
    if (( count > LIMIT )); then
        silted+=("$(dirname "$deps")")
    fi
done

# Named whatever else happens: the prune leaves 30+ GB behind each time and
# nothing tracks that either.
stale=(target/*.stale-*)
if [[ -d ${stale[0]:-} ]]; then
    echo
    echo "Stale copies from earlier prunes, kept rather than deleted:"
    du -sh "${stale[@]}" 2>/dev/null | sed 's/^/  /'
fi

if (( ${#silted[@]} == 0 )); then
    exit 0
fi

if [[ ${1:-} == --prune ]]; then
    for dir in "${silted[@]}"; do
        mv "$dir" "$dir.stale-$(date +%F)"
        echo "moved $dir aside; cargo will rebuild"
    done
    exit 0
fi

echo
echo "Past $LIMIT files, where the exec tax is worth a rebuild. Run:"
echo "    ./scripts/target-silt.sh --prune"
exit 1
