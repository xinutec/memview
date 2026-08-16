#!/usr/bin/env bash
# What is an `if`, to bash? — measured, not remembered.
#
#   nix develop -c bash reader/probes/conditional.sh
#
# `declare -f` renders bash's own parse as text (see bash-printer.sh for why that
# is the second gate). This probe asks it the questions a conditional node has to
# answer — above all whether `elif` is a shape of its own or sugar for a nested
# `if`, which decides whether the tree holds a list of arms or a chain.
#
# Measured 2026-08-16, bash 5.3.15. Six findings, in the order they change the
# design:
#
#   1. `elif` IS DESUGARED, and this is the one that shapes the node.
#      `if a; then b; elif c; then d; fi` comes back as
#      `if a; then b; else if c; then d; fi; fi` — an `elif` is an `else` holding
#      one nested conditional, and bash unfolds every link of a chain that way.
#      So the tree holds a condition, a `then` and ONE optional `else`. A list of
#      arms would make those two texts two trees, and the second gate would say
#      so.
#   2. THE CONDITION IS A LIST, not a command: `if a; b; then c; fi` runs both
#      and branches on `b`. `if a & then b; fi` is legal too, so the list may end
#      backgrounded. Each arm is a list for the same reason.
#   3. AN EMPTY ARM IS A SYNTAX ERROR. `if a; then fi`, `if; then b; fi`,
#      `if a; then b; else fi` and `if a; then b; elif c; fi` are all refused, as
#      is `if a then b; fi` — `then` is only the keyword where a command begins.
#      So a refusal on those shapes is a claim about the INPUT, and `bash -n`
#      adjudicates it.
#   4. A CONDITIONAL IS A COMMAND. `a | if b; then c; fi`, `! if …` and
#      `time if …` are all accepted, so it sits in a pipeline like any other
#      command — and its redirections go after the `fi`, which is where bash
#      prints them.
#   5. A `&` ALREADY TERMINATES ITS LIST. `if a; then b & fi` is legal and
#      `if a; then b & ; fi` is a syntax error, and the same holds of every
#      `do … done`. A printer that always writes `; ` before the closing keyword
#      emits text bash refuses — see section 7, and `print::follow`.
#   6. COMMENTS ARE DELETED, here as everywhere. The parser refuses one inside a
#      conditional rather than dropping it, because the printer writes the whole
#      construct on one line.
set -euo pipefail

render() {
  unset -f __p__ 2>/dev/null
  eval "__p__() {
$1
}" 2>/dev/null || { printf '<<REFUSED>>'; return 1; }
  declare -f __p__ | sed '1,2d;$d;s/^        //'
}

section() { printf '\n=== %s\n' "$1"; }

show() {
  printf -- '--- in\n%s\n--- out\n' "$1"
  if ! once=$(render "$1"); then printf '%s\n' "$once"; return; fi
  printf '%s\n' "$once"
  if ! twice=$(render "$once"); then printf 'TWICE: <<REFUSED>>\n'; return; fi
  [[ $once == "$twice" ]] && printf 'TWICE: fixpoint\n' \
                          || printf 'TWICE: DIFFERS\n%s\n' "$twice"
}

section "1 · the shapes, and how bash lays them out"
for s in \
  'if a; then b; fi' \
  'if a; then b; else c; fi' \
  'if a; then b; elif c; then d; fi' \
  'if a; then b; elif c; then d; else e; fi' \
  'if a; then b; elif c; then d; elif e; then f; else g; fi' \
  'if a; then if b; then c; fi; fi' \
  'if a; then b; else if c; then d; fi; fi'
do show "$s"; done

section "2 · the condition is a LIST, and the arms are too"
for s in \
  'if a; b; then c; fi' \
  'if ! a; then b; fi' \
  'if a && b || c; then d; fi' \
  'if a | b; then c; fi' \
  'if a & then b; fi' \
  'if for f in x; do y; done; then b; fi' \
  'if a; then b; c; d; fi' \
  'if a; then b & fi'
do show "$s"; done

section "3 · redirections, and where they attach"
for s in \
  'if a; then b; fi > out' \
  'if a; then b; fi 2>&1' \
  'if a > c; then b; fi' \
  'if a; then b > out; fi' \
  'if a; then cat; fi <<EOF
body
EOF' \
  'if a; then cat <<EOF
body
EOF
fi'
do show "$s"; done

section "4 · the shapes bash refuses, which fix what a refusal may claim"
for s in \
  'if a; then fi' \
  'if a; then b; else fi' \
  'if; then b; fi' \
  'if a; then b' \
  'if a then b; fi' \
  'if a; then b; elif c; fi' \
  'if a; else b; fi'
do show "$s"; done

section "5 · quoting, and whether a keyword stays one"
for s in \
  "'if' a" \
  'if a; then "then"; fi' \
  'x=if; $x a' \
  'if a; then b; fi | c' \
  'a | if b; then c; fi' \
  '! if a; then b; fi' \
  'time if a; then b; fi'
do show "$s"; done

section "6 · a comment inside the arms"
show 'if a; then
# note
b
fi'

section "7 · a body that ends in & takes no ; after it"
# ⚠ Asked with `bash -n` rather than `declare -f`, because the question is
# whether the text PARSES: this is the one shape the printer can emit that no
# gate would object to. Gate 1 re-reads our own output with our own parser, which
# is more permissive here, and gate 2 is shown the original command by design.
for s in 'if a; then b & fi' 'if a; then b & ; fi' \
         'for f in x; do y & done' 'for f in x; do y & ; done' \
         'while a; do b & done' 'while a; do b & ; done'; do
  printf -- '%-34s ' "$s"
  if bash -n <<<"$s" 2>/dev/null; then echo 'parses'; else echo 'REFUSED'; fi
done
