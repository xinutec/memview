#!/usr/bin/env bash
# Push the Mac's memory corpus up to the memview pod on isis.
#
#   ./scripts/sync.sh [--dry-run]
#
# Direction is not a detail. The Mac is the root of truth for the corpus and the
# only machine on the VPN that nothing else can reach; isis is the exposed,
# disposable mirror. So this pushes, and isis never pulls — a compromised server
# must not be able to reach back and delete the archive. If the copy on isis is
# ever wrong or missing, the fix is always to re-run this, never to recover from
# the server.
#
# Transport is `tar | kubectl exec` rather than rsync because the runtime image
# is debian-slim with no rsync, and adding one to serve a once-a-day sync of a
# megabyte of markdown is not a trade worth making.
set -euo pipefail

MEMORY_DIR="${MEMORY_DIR:-$HOME/.claude/projects/-Users-pippijn-Code/memory}"
HOST="${MEMVIEW_HOST:-root@isis.xinutec.org}"
NAMESPACE=memview
DEPLOY=deploy/memview

dry_run=false
[[ ${1:-} == --dry-run ]] && dry_run=true

[[ -d $MEMORY_DIR ]] || { echo "no corpus at $MEMORY_DIR" >&2; exit 2; }
[[ -f $MEMORY_DIR/MEMORY.md ]] || {
  # The index is what the viewer's front page is built from, and its absence
  # almost always means MEMORY_DIR points somewhere plausible but wrong.
  echo "no MEMORY.md in $MEMORY_DIR — is that really the corpus?" >&2
  exit 2
}

count=$(find "$MEMORY_DIR" -type f -name '*.md' | wc -l | tr -d ' ')
bytes=$(du -sk "$MEMORY_DIR" | cut -f1)
echo "corpus: $count memories, ${bytes}K, from $MEMORY_DIR"

if $dry_run; then
  echo "dry run — would push to $HOST, namespace $NAMESPACE"
  find "$MEMORY_DIR" -type f | sed "s|^$MEMORY_DIR/||" | sort | head -5
  echo "  ..."
  exit 0
fi

remote() { ssh "$HOST" "kubectl -n $NAMESPACE exec -i $DEPLOY -- $*"; }

echo "pushing…"
# Files only, never the directory entry. `tar -c .` archives `./` itself, and
# extracting that tries to restore the archive's mode and timestamps onto
# /corpus — a mounted volume owned by the node, which the container's uid 65532
# may not chmod. tar treats the refusal as fatal even though every file inside
# extracted perfectly, so the sync "failed" with the corpus fully copied.
#
# --no-same-owner because a non-root extract cannot chown and should not try.
( cd "$MEMORY_DIR" && find . -type f -print0 | tar --null -T - -czf - ) \
  | remote sh -c "'tar -C /corpus -xzf - --no-same-owner'"

# Prune what is no longer in the corpus. Without this a deleted memory would
# live on in the viewer forever — and a memory gets deleted precisely when it
# has turned out to be wrong, which is the worst thing to leave on display.
#
# The keep-list is written to /state, the one writable mount: the container runs
# with a read-only root filesystem, so /tmp is not available.
echo "pruning…"
find "$MEMORY_DIR" -type f | sed "s|^$MEMORY_DIR/||" | remote sh -c "'
  set -e
  cat > /state/.sync-keep
  cd /corpus
  find . -type f | sed \"s|^\./||\" | while read -r f; do
    grep -qxF \"\$f\" /state/.sync-keep || rm -f \"\$f\"
  done
  rm -f /state/.sync-keep
'"

# The co-use artefact, if it has been mined. Pushed to /state, NOT /corpus: the
# prune above deletes anything in /corpus that is not a memory on the Mac, and
# this is not a memory. It carries only names and integers — no transcript text
# ever reaches it — but it does describe working patterns, which is why it goes
# to a VPN-only, owner-gated app and nowhere else.
COUSE="${COUSE_FILE:-$(dirname "$MEMORY_DIR")/couse.json}"
if [[ -f $COUSE ]]; then
  echo "pushing co-use…"
  remote sh -c "'cat > /state/couse.json'" < "$COUSE"
else
  echo "no co-use artefact at $COUSE — skipping (mine it with: cargo run --release --bin couse)"
fi

echo "verifying…"
remote sh -c "'ls /corpus | wc -l'" | tr -d '\r' | while read -r n; do
  echo "  $n files on isis"
done
echo "done — https://memview.xinutec.org"
