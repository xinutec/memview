#!/usr/bin/env bash
# What is a `${…}` operator, to bash? — measured, not remembered.
#
#   nix develop -c bash reader/probes/parameter-op.sh
#
# `declare -f` renders bash's own parse as text (see bash-printer.sh for why
# that is the second gate). This probe asks it the questions the parameter
# operator node has to answer — and the FIRST answer is that this gate cannot
# help, which is what makes the rest of them load-bearing.
#
# Measured 2026-08-16, bash 5.3.15. Five findings, in the order they change the
# design:
#
#   1. EVERY OPERATOR FORM COMES BACK VERBATIM. `${x:-y}`, `${x%%.*}`,
#      `${x/a/b}`, `${x:1:3}`, `${PIPESTATUS[0]}` — all of them, exactly as
#      written. So the second gate compares two identical texts and has NO
#      opinion about what is inside the braces, precisely as it has none about
#      the inside of a word. The round-trip law and construction are the whole
#      defence: an operator absorbed into a literal would satisfy both gates.
#   2. THE COLON IS SEMANTIC. `${x-y}` and `${x:-y}` are printed apart and mean
#      different things — unset versus unset-or-empty — so it is a field on the
#      node rather than a spelling.
#   3. THE OPERAND NESTS. `${x:-$(date)}` and `${x:-${y}}` are legal, so the
#      operand cannot be found by scanning to the first `}`.
#   4. THE OPERAND HOLDS SPACES BARE. `${x:-a b}` is ONE word; the braces are
#      the delimiter, so nothing inside needs quoting to stay together.
#   5. A SUBSCRIPT IS PART OF NAMING THE VALUE. `${a[0]}` and `$a[0]` are
#      different words — the second is `$a` followed by the literal `[0]` — so
#      the printer must brace whenever a subscript is present.
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
  printf -- '%-30s -> ' "$1"
  if ! once=$(render "$1"); then printf '%s\n' "$once"; return; fi
  printf '%s' "$once"
  twice=$(render "$once" 2>/dev/null) || { printf '   TWICE: <<REFUSED>>\n'; return; }
  [[ $once == "$twice" ]] && printf '\n' || printf '   TWICE DIFFERS: %s\n' "$twice"
}

section "1 · every form, and whether bash keeps the spelling"
for s in 'echo ${x:-y}' 'echo ${x-y}' 'echo ${x:=y}' 'echo ${x=y}' 'echo ${x:?y}' \
         'echo ${x:+y}' 'echo ${x+y}' 'echo ${#x}' 'echo ${!x}' \
         'echo ${x#p}' 'echo ${x##p}' 'echo ${x%s}' 'echo ${x%%s}' \
         'echo ${x/a/b}' 'echo ${x//a/b}' 'echo ${x/#a/b}' 'echo ${x/%a/b}' 'echo ${x/a}' \
         'echo ${x^}' 'echo ${x^^}' 'echo ${x,}' 'echo ${x,,}' 'echo ${x:1:3}'; do
  show "$s"
done

section "2 · the subscript, and why the braces are not optional"
for s in 'echo ${a[0]}' 'echo ${a[@]}' 'echo ${a[*]}' 'echo ${a[$i]}' 'echo ${#a[@]}' \
         'echo $a[0]' 'echo "${a[0]}"'; do
  show "$s"
done

section "3 · what the operand may hold"
for s in 'echo ${x:-$(date)}' 'echo ${x:-${y}}' 'echo ${x:-a b}' 'echo ${x:-"a b"}' \
         'echo ${x:-}' 'echo ${x:-*}' 'echo ${x:-a/b}' 'echo ${x:-}' \
         'echo ${x#*/}' 'echo ${f%%.*}' 'echo ${x:-\}}'; do
  show "$s"
done

section "4 · the shapes bash refuses"
for s in 'echo ${x' 'echo ${}' 'echo ${x:*}' 'echo ${a[}'; do show "$s"; done
