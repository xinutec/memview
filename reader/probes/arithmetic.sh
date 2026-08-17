#!/usr/bin/env bash
# What is arithmetic, to bash? — measured, not remembered.
#
#   nix develop -c bash reader/probes/arithmetic.sh
#
# Measured 2026-08-17, bash 5.3.15. Four findings:
#
#   1. ⚠ **VERBATIM, whitespace included.** `$(( 1 + 2 ))` comes back with its
#      spaces. So the second gate has NO opinion about an expression — it
#      compares two identical texts — and a reader that kept the source between
#      the parens would satisfy the round-trip law while saying nothing true.
#      This is the construct where "nothing is left unparsed" has to be taken
#      literally: the tree holds an expression, and the printer rebuilds the
#      parens from precedence.
#   2. `((…))` IS NOT TWO SUBSHELLS. `((a))` evaluates; `( (a) )` runs a command
#      called `a`. The parser has to check for the doubled paren first.
#   3. THE C-STYLE `for` IS NORMALISED, unlike the expression itself:
#      `for ((i=0;i<3;i++))` prints as `for ((i=0; i<3; i++))`. So the parser
#      must read that spacing back — gate 2 hands it exactly that text.
#   4. A BASE PREFIX IS PART OF THE NUMBER. `10#$m` reads the expansion's digits
#      in base 10, which is how the corpus turns a zero-padded `08` into eight —
#      `$((08))` is an invalid octal and `$((10#08))` is 8. Three commands
#      refused until the node held it.
set -euo pipefail

render() {
  unset -f __p__ 2>/dev/null
  eval "__p__() {
$1
}" 2>/dev/null || { printf '<<REFUSED>>'; return 1; }
  declare -f __p__ | sed '1,2d;$d;s/^        //' | tr '\n' '|'
}

section() { printf '\n=== %s\n' "$1"; }

section "1 · what bash's printer keeps"
for s in 'echo $((1+2))' 'echo $(( 1 + 2 ))' 'echo $((a*b))' 'echo $(($x+1))' 'echo $((x++))' \
         '((i++))' '((i=1))' 'for ((i=0;i<3;i++)); do echo $i; done' \
         'echo $((0x1f))' 'echo $((a?b:c))' 'echo $((a,b))' 'echo $(( (1+2)*3 ))' \
         'echo $((10#$m))' 'echo $[1+2]'; do
  printf -- '%-38s -> %s\n' "$s" "$(render "$s")"
done

section "2 · (( )) is arithmetic, not two subshells"
# ⚠ The first prints an arithmetic command; the second runs a command called `a`
# and would fail with "command not found" if it ran. Different constructs.
printf -- '%-14s -> %s\n' '((a))'    "$(render '((a))')"
printf -- '%-14s -> %s\n' '( (a) )'  "$(render '( (a) )')"

section "3 · what a base prefix is worth"
for s in '$((08))' '$((10#08))' '$((16#ff))' '$((2#101))'; do
  printf -- '%-12s -> ' "$s"
  # `$((08))` is an invalid octal and errors — which IS the finding, so the
  # failure is expected rather than fatal.
  (eval "echo $s") 2>&1 | head -1 || true
done
