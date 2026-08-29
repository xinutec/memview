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

# Push one derived artefact: compressed on the wire, renamed into place.
#
# ⚠ **Compressed, and the corpus beside it has been since the day this was
# written.** These are JSON with the same field names on every row, which is
# exactly what a compressor eats: measured 2026-08-14, effects.json goes 35.2 MB
# → 4.15 MB, 8.5×, in 0.65s. The alternative on the table was re-encoding its
# rows as positional arrays — that saves 6.7 MB raw and, once both are gzipped,
# **0.10 MB, 2.4%**, in exchange for a format where adding a field silently
# reinterprets every row (#858). The expensive thing was never the encoding; it
# was that all four of these skipped the compression `tar -czf` already gives
# /corpus — 61 MB of JSON going up uncompressed beside a tarred corpus.
#
# ⚠ **Written to a temp and renamed**, because the app re-reads these on demand
# and truncate-then-write leaves a window in which the file on disk is not valid
# JSON — [[reference_write_then_rename_or_the_reader_sees_half]]. Both names are
# on /state, so the rename is within one filesystem and therefore atomic.
push_json() {
  local what=$1 from=$2 to=$3
  # ⚠ `${what}`, braced. `"$what…"` made bash read the ellipsis's bytes as part
  # of the NAME and die on `what…: unbound variable` — the high bytes count as
  # alphabetic in this locale. Every other message here is a literal, so this is
  # the first line in the file where a variable met one of its own ellipses.
  echo "pushing ${what}…"
  gzip -c "$from" | remote sh -c "'gzip -dc > $to.new && mv $to.new $to'"
}

# The co-use artefact, if it has been mined. Pushed to /state, NOT /corpus: the
# prune above deletes anything in /corpus that is not a memory on the Mac, and
# this is not a memory. It carries only names and integers — no transcript text
# ever reaches it — but it does describe working patterns, which is why it goes
# to a VPN-only, owner-gated app and nowhere else.
COUSE="${COUSE_FILE:-$(dirname "$MEMORY_DIR")/couse.json}"
if [[ -f $COUSE ]]; then
  push_json co-use "$COUSE" /state/couse.json
else
  echo "no co-use artefact at $COUSE — skipping (mine it with: cargo run --release --bin couse)"
fi

# The agent roster: which named session works in which project directory.
# Names, project names and integers, same as the co-use artefact.
AGENTS="${AGENTS_FILE:-$HOME/.claude/memview/cache/agents.json}"
if [[ -f $AGENTS ]]; then
  push_json agents "$AGENTS" /state/agents.json
else
  echo "no agent artefact at $AGENTS — skipping (mine it with: cargo run --release --bin agents)"
fi

# The timeline: what each session did, in order, and how it turned out. Derived
# and typed — an agent, a moment, a repository, a kind of work, a verdict — and
# carrying no command line, no prompt and no output text. Pippijn lifted the
# no-timeline rule on 2026-08-02 and left that half of it standing.
DOING="${DOING_FILE:-$HOME/.claude/memview/cache/doing.json}"
if [[ -f $DOING ]]; then
  push_json timeline "$DOING" /state/doing.json
else
  echo "no timeline at $DOING — skipping (mine it with: cargo run --release --bin agents)"
fi

# What each turn did to which file, with the command that did it — the evidence
# under a timeline row. The largest thing this pushes, ~35 MB.
EFFECTS="${EFFECTS_FILE:-$HOME/.claude/memview/cache/effects.json}"
if [[ -f $EFFECTS ]]; then
  push_json effects "$EFFECTS" /state/effects.json
else
  echo "no effects at $EFFECTS — skipping (mine it with: cargo run --release --bin agents)"
fi

# ⚠ **COMMAND TEXT IS PUSHED NOW, and until 2026-08-13 none was.** The note that
# stood here said "no transcript TEXT is pushed, and there is no artefact
# carrying any". Half of that is no longer true and the half that is still holds,
# so it is worth being exact about which.
#
# What changed: Pippijn settled the trust question — "Isis should be trusted.
# Everything can go there." — so effects.json carries each command verbatim.
# Prompts, replies and command OUTPUT are still not mined and still have no
# artefact; the only text that travels is the command line itself.
#
# What did NOT change, and is the reason this stayed a derived artefact rather
# than becoming the old `history.json` back: memview is for reading the memory
# documents well, and a viewer that also served the literal history makes the
# corpus depend on the transcripts instead of distilling them. That argument was
# never about privacy and trust does not settle it. So a command travels
# *because a claim needs the command it rests on* — effects.json is keyed by
# (agent, minute) to the timeline, holds no session id, and is in no order a
# conversation could be read from.

echo "verifying…"
remote sh -c "'ls /corpus | wc -l'" | tr -d '\r' | while read -r n; do
  echo "  $n files on isis"
done
echo "done — https://memview.xinutec.org"
