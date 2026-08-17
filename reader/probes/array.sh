#!/usr/bin/env bash
# What is `x=(a b)` to bash? — measured, not remembered.
#
#   nix develop -c bash reader/probes/array.sh
#
# It was hiding: 16 of the 28 commands the report filed under `grouping` are
# array assignments, which is not a grouping at all. A reason is a unit of work,
# and this one had two constructs in it.
#
# Measured 2026-08-17, bash 5.3.15. Five findings, in the order they change the
# design:
#
#   1. THE SECOND GATE CAN SEE INSIDE ONE. Bash NORMALISES the whitespace
#      between elements — `x=(a   b)` comes back as `x=(a b)`, and one written
#      across four lines comes back on one — so it is a real parse on both
#      sides, unlike the inside of a word. A mis-split of the elements would be
#      caught.
#   2. IT IS LEGAL IN EXACTLY TWO PLACES, and section 2 puts both to `bash -n`:
#      as a command PREFIX (`x=(a) cmd`), and as an argument to a declaration
#      builtin — `declare`, `typeset`, `export`, `readonly`, `local`. `echo
#      x=(a)` is a SYNTAX ERROR, so the tree may not read one anywhere else.
#   3. AN ELEMENT MAY CARRY A KEY. `x=([0]=a [1]=b)` and `declare -A M=([k]=v)`
#      come back verbatim, and the corpus holds both — so an element is a
#      key/value pair with the key optional, not a bare word.
#   4. A NEWLINE BETWEEN ELEMENTS IS A SEPARATOR, not a terminator: the
#      multi-line shape below is one assignment.
#   5. `+=` APPENDS to an array as it does to a string, and the spelling
#      survives.
set -euo pipefail

render() {
  unset -f __p__ 2>/dev/null
  eval "__p__() {
$1
}" 2>/dev/null || { printf '<<REFUSED>>'; return 1; }
  declare -f __p__ | sed '1,2d;$d;s/^    //'
}

section() { printf '\n=== %s\n' "$1"; }

section "1 · what comes back, and is it a fixpoint"
shapes=(
  'x=(a b c)'
  'x=()'
  'x+=(d)'
  'x=(a   b)'
  'x=("a b" c)'
  'x=($(ls) *.txt)'
  'x=([0]=a [1]=b)'
  'declare -a T=(a b)'
  'declare -A M=([k]=v)'
  'local -a q=(1 2)'
  'x=(a b) y=(c)'
  'x=(a b) cmd arg'
  # written across lines, which is how the corpus spells a long one
  'moves=(
"one two"
"three"
)'
)
for s in "${shapes[@]}"; do
  printf -- '--- in  %s\n    out ' "$s"
  if ! once=$(render "$s"); then printf '%s\n' "$once"; continue; fi
  printf '%s' "$once" | tr '\n' '⏎'
  if ! twice=$(render "$once"); then printf ' | TWICE REFUSED\n'; continue; fi
  if [[ $once == "$twice" ]]; then
    printf ' | fixpoint\n'
  else
    printf ' | DIFFERS '
    printf '%s' "$twice" | tr '\n' '⏎'
    printf '\n'
  fi
done

section "2 · where one is legal at all"
# ⚠ The whole reason the parser needs the command NAME: bash decides by it.
for s in 'x=(a) cmd' 'declare x=(a b)' 'typeset x=(a)' 'export x=(a)' \
         'readonly x=(a)' 'local x=(a)' 'A=1 declare x=(a)' \
         'echo x=(a)' 'cmd x=(a b)' 'echo "x=(a)"'; do
  printf -- '--- %-22s ' "$s"
  if printf '%s\n' "$s" | bash -n 2>/dev/null; then
    printf 'bash -n ACCEPTS\n'
  else
    printf 'bash -n refuses\n'
  fi
done
