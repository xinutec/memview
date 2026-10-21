#!/usr/bin/env bash
# Push the Mac's memory corpus to the memview pod on isis.
#
#   ./scripts/sync.sh [--dry-run]
#
# Push only: the Mac is the root of truth and isis a disposable mirror, so a
# compromised server must have no way back. If isis's copy is wrong, re-run this.
#
# `tar | kubectl exec`, not rsync: the runtime image has no rsync.
set -euo pipefail

# Claude Code names a project directory after its path, every `/` a `-`.
project="$HOME/.claude/projects/$(printf %s "$HOME/Code" | tr -c 'A-Za-z0-9-' -)"
MEMORY_DIR="${MEMORY_DIR:-$project/memory}"
HOST="${MEMVIEW_HOST:-root@isis.xinutec.org}"
NAMESPACE=memview
DEPLOY=deploy/memview

dry_run=false
[[ ${1:-} == --dry-run ]] && dry_run=true

[[ -d $MEMORY_DIR ]] || { echo "no corpus at $MEMORY_DIR" >&2; exit 2; }
[[ -f $MEMORY_DIR/MEMORY.md ]] || {
  # No index almost always means MEMORY_DIR points at the wrong directory.
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
# Files only: archiving `.` makes tar restore its mode onto /corpus, a mount the
# container's uid may not chmod, and tar then fails with every file copied.
# A non-root extract cannot chown, hence --no-same-owner.
( cd "$MEMORY_DIR" && find . -type f -print0 | tar --null -T - -czf - ) \
  | remote sh -c "'tar -C /corpus -xzf - --no-same-owner'"

# Prune what the corpus no longer has: a memory is deleted when it proved wrong,
# the worst thing to leave on display. The keep-list goes to /state, the only
# writable mount.
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

# Push one derived artefact, gzipped (repetitive JSON shrinks several-fold) and
# renamed into place: the app re-reads these, and a half-written file is not
# JSON. Both names are on /state, so the rename is atomic.
push_json() {
  local what=$1 from=$2 to=$3
  # Braced: bash reads the ellipsis's high bytes as part of the name.
  echo "pushing ${what}…"
  gzip -c "$from" | remote sh -c "'gzip -dc > $to.new && mv $to.new $to'"
}

# Co-use counts, if mined. To /state, not /corpus: the prune deletes anything in
# /corpus that is not a memory.
COUSE="${COUSE_FILE:-$(dirname "$MEMORY_DIR")/couse.json}"
if [[ -f $COUSE ]]; then
  push_json co-use "$COUSE" /state/couse.json
else
  echo "no co-use artefact at $COUSE — skipping (mine it with: cargo run --release --bin couse)"
fi

# Which named session works in which project directory.
AGENTS="${AGENTS_FILE:-$HOME/.claude/memview/cache/agents.json}"
if [[ -f $AGENTS ]]; then
  push_json agents "$AGENTS" /state/agents.json
else
  echo "no agent artefact at $AGENTS — skipping (mine it with: cargo run --release --bin agents)"
fi

# What each session did, in order, and how it turned out; typed fields only, no
# command, prompt or output text.
DOING="${DOING_FILE:-$HOME/.claude/memview/cache/doing.json}"
if [[ -f $DOING ]]; then
  push_json timeline "$DOING" /state/doing.json
else
  echo "no timeline at $DOING — skipping (mine it with: cargo run --release --bin agents)"
fi

# What each turn did to which file, with the command that did it: the evidence
# under a timeline row, and the largest push.
EFFECTS="${EFFECTS_FILE:-$HOME/.claude/memview/cache/effects.json}"
if [[ -f $EFFECTS ]]; then
  push_json effects "$EFFECTS" /state/effects.json
else
  echo "no effects at $EFFECTS — skipping (mine it with: cargo run --release --bin agents)"
fi

# Command lines travel (isis is trusted with everything); prompts, replies and
# output are not mined at all. effects.json stays derived, keyed to the timeline
# with no session id or conversation order, because memview distils the
# transcripts rather than serving them.

echo "verifying…"
remote sh -c "'ls /corpus | wc -l'" | tr -d '\r' | while read -r n; do
  echo "  $n files on isis"
done
echo "done — https://memview.xinutec.org"
