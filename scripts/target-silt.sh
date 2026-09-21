#!/usr/bin/env bash
# How many files the build tree holds, failing past the threshold.
#
#   ./scripts/target-silt.sh            # report
#   ./scripts/target-silt.sh --prune    # move the silted directories aside
#
# Cargo never prunes stale artefacts and exec cost is O(files in the directory),
# so a repo gated on every commit silts up its own build tree and then pays for
# it on the first run of each freshly written binary.
#
# Moved rather than deleted: `mv` is atomic and cargo rebuilds, where deleting
# tens of gigabytes under a running build is neither.
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
    # An `if`, not `(( … )) && …`: a false arithmetic test returns 1, which under
    # `set -e` is exempt only while it is not the last command of the AND-list.
    if (( count > LIMIT )); then
        silted+=("$(dirname "$deps")")
    fi
done

# Names only. Sizing them walks tens of gigabytes and takes twenty seconds, which
# this would pay on every commit; `du -sh target/*.stale-*` when you want it.
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
        mv "$dir" "$dir.stale-$(date +%F)"
        echo "moved $dir aside; cargo will rebuild"
    done
    exit 0
fi

echo
echo "Past $LIMIT files, where the exec tax is worth a rebuild. Run:"
echo "    ./scripts/target-silt.sh --prune"
exit 1
