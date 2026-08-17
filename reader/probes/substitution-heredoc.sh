#!/usr/bin/env bash
# A heredoc inside `$( )` — where does its body go? — measured, not remembered.
#
#   nix develop -c bash reader/probes/substitution-heredoc.sh
#
# This is the one construct in the corpus whose operand cannot be printed on the
# line that asks for it: `x=$(cat <<EOF … EOF)` opens a heredoc inside a word,
# and the printer writes words inline. The question this probe settles is what
# bash itself does with the shape, because matching bash is what makes the
# second gate able to compare trees at all.
#
# Measured 2026-08-17, bash 5.3.15. Five findings, in the order they change the
# design:
#
#   1. BASH PRINTS IT ACROSS LINES, and that print is a fixpoint. `declare -f`
#      renders the substitution with the body and terminator on their own lines,
#      exactly where they were written, inside the word. So "one command per
#      line" is not bash's rule either — the printer may spell this shape the
#      way bash does, and the second gate then has something to compare.
#   2. THE INNER BODY IS READ AT THE INNER NEWLINE, before the enclosing line
#      ends. `f "$(cat <<X … X)" <<A` gives the argument `X`'s body and stdin
#      `A`'s — and so does `f <<A "$(cat <<X … X)"`, where A is written FIRST.
#      So the pairing is by the order the bodies were read, which puts
#      everything inside a substitution ahead of everything the enclosing line
#      opened, whichever order the openers appear in. Section 2 runs all three
#      positions — argument, argument-after-the-opener, redirection target — and
#      all three agree.
#   3. SO NO WALK OVER THE FINISHED TREE REPRODUCES IT. That order is neither
#      the openers' nor the tree's, which is why the parser drains a
#      substitution's bodies at its closing paren and leaves the outer walk only
#      what the outer line opened.
#   4. AN UNTERMINATED INNER HEREDOC IS A WARNING, NOT AN ERROR — `command
#      substitution: N unterminated here-document` on stderr, an empty body, and
#      a zero exit. `bash -n` accepts it. It is the same shape as a runaway
#      heredoc at top level and gets the same treatment: read, not refused.
#   5. A BODY THAT RUNS PAST THE `)` SWALLOWS IT. The `)` becomes body text and
#      the substitution never closes, which bash then reports as a syntax error
#      at the end of the input. Refusing it as an unterminated expansion is the
#      same reading.
set -euo pipefail

# The body of `f`, as bash prints it — see reader/probes/bash-printer.sh, which
# this shares its method with.
render() {
  unset -f __p__ 2>/dev/null
  eval "__p__() {
$1
}" 2>/dev/null || { printf '<<REFUSED>>'; return 1; }
  declare -f __p__ | sed '1,2d;$d;s/^    //'
}

section() { printf '\n=== %s\n' "$1"; }

section "1 · what bash prints, and whether it is a fixpoint"
shapes=(
  # the two spellings the corpus actually holds: an assignment, and a quoted
  # substitution handed to a command as one argument
  'moved=$(python3 - <<PY
print("x")
PY
)'
  'task add "$(cat <<'"'"'EOF'"'"'
body
EOF
)"'
  # the delimiter forms, inside
  'x=$(cat <<-EOF
	tabbed
	EOF
)'
  # two heredocs inside one substitution, and one inside each of two
  'x=$(cat <<A <<B
first
A
second
B
)'
  'x=$(cat <<A
first
A
)$(cat <<B
second
B
)'
  # nested one level down
  'x=$(echo "$(cat <<X
deep
X
)")'
  # inside a compound, which the printer writes on ONE line — does the body
  # still find its place?
  'for f in a; do x=$(cat <<X
body
X
); echo "$x"; done'
  # an outer heredoc as well as an inner one, both orders
  'f "$(cat <<X
inner
X
)" <<A
outer
A'
  'f <<A "$(cat <<X
inner
X
)"
outer
A'
)
for s in "${shapes[@]}"; do
  printf -- '--- in\n%s\n--- out\n' "$s"
  if ! once=$(render "$s"); then printf '%s\n' "$once"; continue; fi
  printf '%s\n' "$once"
  if ! twice=$(render "$once"); then printf 'TWICE: <<REFUSED>>\n'; continue; fi
  [[ $once == "$twice" ]] && printf 'TWICE: fixpoint\n' \
                          || printf 'TWICE: DIFFERS\n%s\n' "$twice"
done

section "2 · which body goes to which opener, run for real"
# ⚠ The pairing is what a positional walk over the tree has to reproduce, and
# printing it is the only way to know it rather than argue it.
show() { printf 'arg=<%s> stdin=<%s>\n' "$1" "$(cat)"; }

printf -- '--- arg first, then the outer opener\n'
show "$(cat <<X
INNER
X
)" <<A
OUTER
A

printf -- '--- outer opener first, then the arg\n'
show <<A "$(cat <<X
INNER
X
)"
OUTER
A

printf -- '--- the substitution in a redirection TARGET\n'
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT
cat > "$scratch/$(cat <<X
name
X
)" <<A
OUTER
A
printf 'wrote <%s> holding <%s>\n' "$(ls "$scratch")" "$(cat "$scratch"/*)"

section "3 · the degenerate shapes"
# Each is adjudicated by bash itself: what it says, and whether `bash -n` — the
# third gate — calls it shell at all.
degenerate=(
  'x=$(cat <<X); echo "[$x]"'
  'x=$(cat <<X
body
); echo "[$x]"'
  'x=$(cat <<X
body
X
); echo "[$x]"'
)
for s in "${degenerate[@]}"; do
  printf -- '--- in\n%s\n--- ' "$s"
  if printf '%s\n' "$s" | bash -n 2>/dev/null; then
    printf 'bash -n ACCEPTS · '
  else
    printf 'bash -n refuses · '
  fi
  # ⚠ The exit code is half the finding — a warning leaves it 0 and a syntax
  # error makes it 2 — so it is captured rather than allowed to end the script.
  out=$(printf '%s\n' "$s" | bash 2>&1) && status=0 || status=$?
  printf 'exit %d · %s\n' "$status" "$(printf '%s' "$out" | tr '\n' '⏎')"
done
