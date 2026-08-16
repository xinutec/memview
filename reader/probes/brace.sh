#!/usr/bin/env bash
# What is a brace expansion, to bash? — measured, not remembered.
#
#   nix develop -c bash reader/probes/brace.sh
#
# Two questions, and the second is the one that shapes the node: what does
# `declare -f` keep of a brace expansion (nothing — it is printed verbatim, so
# the second gate has no opinion), and WHICH braces expand at all.
#
# Measured 2026-08-17, bash 5.3.15. Four findings:
#
#   1. VERBATIM, every form. `{a,b}`, `{1..9..2}`, `{a..e}`, `{a,{b,c}}` all come
#      back as written, so gate 2 cannot see inside one — the same blindness it
#      has about a word and about `${…}`. Construction and the round-trip law
#      are the whole defence.
#   2. ⚠ **A brace with nothing to expand is ORDINARY TEXT.** `{a}` and `{}` are
#      printed and expanded as themselves. So reading them as literal characters
#      is what bash does, not a construct being swallowed — and it is why the
#      decision is a lookahead made before anything is consumed.
#   3. IT CHANGES HOW MANY WORDS THERE ARE. `a{b,c}d` is one word that becomes
#      `abd acd`, which is why it sits beside a glob rather than beside a
#      compound statement. Quoting suppresses it entirely: `"{a,b}"` is one word.
#   4. A RANGE IS DIGITS OR SINGLE LETTERS, and may descend: `{2..0}` gives
#      `2 1 0`. Anything else holding two dots is text bash leaves alone.
set -euo pipefail

render() {
  unset -f __p__ 2>/dev/null
  eval "__p__() {
$1
}" 2>/dev/null || { printf '<<REFUSED>>'; return 1; }
  declare -f __p__ | sed '1,2d;$d;s/^        //'
}

section() { printf '\n=== %s\n' "$1"; }

section "1 · what bash's printer keeps (the gate-2 view)"
for s in 'echo {a,b}' 'echo a{b,c}d' 'echo {1..3}' 'echo {1..9..2}' 'echo {a..e}' \
         'echo {a,{b,c}}' 'echo {a}' 'echo {}' 'echo {a,}' 'echo "{a,b}"' 'echo ${x}{a,b}'; do
  printf -- '%-24s -> %s\n' "$s" "$(render "$s")"
done

section "2 · which braces actually expand, and into how many words"
for s in '{a,b}' 'a{b,c}d' '{1..3}' '{a}' '{}' '{a,}' '"{a,b}"' "'{a,b}'" \
         '{1..5..2}' '{a..c}' '{2..0}' '{a,b}{c,d}' '{x..y..z}' '{1.5..3}'; do
  printf -- '%-14s -> ' "$s"
  eval "printf '[%s] ' $s"
  echo
done
