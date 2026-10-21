#!/usr/bin/env bash
# Claude Code PostToolUse hook: after a session touches the memory directory, tell
# it about any index line not yet committed whose memory has no `role:`.
#
# commit-msg refuses this only for a session committing its own memory; most
# leave theirs to the nightly, which only warns. Bash is watched too: sessions
# edit memories with `sed` and heredocs. Never blocks; silent when all is judged.
set -euo pipefail

# The project directory Claude Code keeps for ~/Code: its path with `/` as `-`.
PROJECT="projects/$(sed 's#/#-#g' <<< "$HOME/Code")"
MEMORY="$HOME/.claude/$PROJECT/memory"
input="$(cat)"
case "$(jq -r '.tool_name // empty' <<< "$input")" in
  Write | Edit)
    path="$(jq -r '(.tool_response.filePath // .tool_input.file_path) // empty' <<< "$input")"
    [[ $path == "$MEMORY"/* ]] || exit 0
    ;;
  Bash)
    command="$(jq -r '.tool_input.command // empty' <<< "$input")"
    [[ $command == *"$MEMORY"* || $command == *MEMORY.md* || $command == *"$PROJECT/memory"* ]] || exit 0
    ;;
  *) exit 0 ;;
esac

UNJUDGED="${MEMVIEW_REPO:-$(cd "$(dirname "$0")/.." && pwd)}/target/release/memory-unjudged"
[[ -x $UNJUDGED ]] || UNJUDGED="$HOME/.local/bin/memory-unjudged"
[[ -x $UNJUDGED && -f $MEMORY/MEMORY.md ]] || exit 0

before="$(mktemp)"
trap 'rm -f "$before"' EXIT
git -C "$HOME/.claude" show "HEAD:$PROJECT/memory/MEMORY.md" > "$before" 2>/dev/null || : > "$before"
names="$("$UNJUDGED" --before "$before" --dir "$MEMORY" < "$MEMORY/MEMORY.md")" && exit 0
[[ -n $names ]] || exit 0

note="These memories have an index line not yet committed and no \`role:\` in their frontmatter: $(paste -sd, - <<< "$names" | sed 's/,/, /g'). Add \`role: tripwire\` (read before the moment) or \`role: pointer\` (looked up when needed) to each — see feedback_pointer_or_tripwire. An unjudged indexed memory is held from demotion forever."
jq -n --arg note "$note" '{hookSpecificOutput: {hookEventName: "PostToolUse", additionalContext: $note}}'
