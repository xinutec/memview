#!/usr/bin/env bash
# What is a group, to bash? — measured, not remembered.
#
#   nix develop -c bash reader/probes/grouping.sh
#
# `declare -f` renders bash's own parse as text (see bash-printer.sh for why
# that is the second gate). This probe asks it what separates the four things
# that share `(`, `)`, `{` and `}`: a subshell, a brace group, a function
# definition, and a brace EXPANSION, which is none of the above.
#
# Measured 2026-08-17, bash 5.3.15. Five findings, in the order they change the
# design:
#
#   1. `{ a }` IS A SYNTAX ERROR and `( a )` is not. A brace group's `}` is a
#      reserved word, so the last command must be terminated before it — the
#      printer writes `{ a; }`, and `{ a & }` needs no `;` because the `&`
#      already ended the list.
#   2. THE DEFINITION SPELLING IS NOT KEPT. `f() { a; }` comes back as
#      `function f () { a; }`, so bash has one canonical form and the tree holds
#      no spelling. ⚠ The parser must therefore READ `function NAME`, or it
#      cannot read back its own print: 141 commands failed the round-trip law on
#      exactly that.
#   3. A `( … )` FUNCTION BODY IS WRAPPED IN A BRACE GROUP. `f() ( a )` prints as
#      `function f () { ( a ) }`, so both spellings are one tree — the same
#      collapse `elif` gets.
#   4. `{` IS THE KEYWORD ONLY WHERE A WORD CANNOT START. `{ a; }` is a group and
#      `{a,b}` is one word that expands to two. They share a character and
#      nothing else, which is why they are separate reasons.
#   5. CLOSING A DESCRIPTOR HAS NO DIRECTION. `3<&-` prints as `3>&-` and `<&-`
#      as `0>&-`: one operation, whichever way it was written — though the
#      direction still decides WHICH descriptor. Found by the second gate on one
#      command in 129,329, because our print of the wrong tree read back as the
#      same wrong tree.
set -euo pipefail

render() {
  unset -f __p__ 2>/dev/null
  eval "__p__() {
$1
}" 2>/dev/null || { printf '<<REFUSED>>'; return 1; }
  declare -f __p__ | sed '1,2d;$d;s/^        //' | tr '\n' '|'
}

show() { printf -- '%-24s -> %s\n' "$1" "$(render "$1")"; }

section() { printf '\n=== %s\n' "$1"; }

section "1 · the four shapes, and what bash keeps of each"
for s in '( a; b )' '{ a; b; }' '{ a; }' '{ a }' '( a )' \
         'f() { a; }' 'function f { a; }' 'function f () { a; }' 'f () { a; }' 'f() ( a )'; do
  show "$s"
done

section "2 · where the terminator is required"
for s in '{ a & }' '( a & )' '{ a; } > out' '( a ) > out' '( a; b ) | c' '{ a; } | c'; do
  show "$s"
done

section "3 · a brace group is not a brace expansion"
for s in 'echo {a,b}' 'echo {a,b}.txt' 'echo {1..3}' '{a,b} arg' '{ echo a; }'; do
  show "$s"
done

section "4 · closing a descriptor, which has no direction"
for s in 'exec 3<&-' 'exec 3>&-' 'exec 3<&- 3>&-' 'cat <&-' 'cat >&-' 'exec {v}<&-'; do
  show "$s"
done

section "5 · \$( ) holding a subshell needs the space bash needs"
# ⚠ `$((` opens an ARITHMETIC expansion, so a substitution whose body starts
# with a subshell must be written `$( ( … ) )`. The printer emitted `$((` for 9
# commands, and the round-trip law caught it.
for s in 'echo $( (cd /tmp) && echo ok )' 'echo $((1+2))'; do show "$s"; done
