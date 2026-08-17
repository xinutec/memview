#!/usr/bin/env bash
# What is `[…]` inside a word to bash? — measured, not remembered.
#
#   nix develop -c bash reader/probes/bracket.sh
#
# A bracket expression is a GLOB, so the question it has to answer is not what
# bash prints — it prints every one of them verbatim, exactly as it prints a
# word — but which texts actually name a set. That is a matching question, so
# this probe answers it by matching, against real files in a scratch directory
# it removes.
#
# Measured 2026-08-17, bash 5.3.15. Six findings, in the order they change the
# design:
#
#   1. `[^a]` AND `[!a]` ARE THE SAME SET. Both matched everything but `a`,
#      including the caret itself — so `^` negates rather than joining the set,
#      the two spellings are ONE tree, and the printer may pick either. It picks
#      `!`, which is the POSIX one.
#   2. A `]` IN FIRST POSITION IS A MEMBER, not the close: `[]a]` matches `]`
#      and `a`. Which is also why `[]` and `[!]` are LITERAL — the `]` is
#      consumed as a member and nothing closes them.
#   3. NO `]` AT ALL MEANS LITERAL TEXT. `[abc` expands to itself, so an
#      unclosed bracket is not a refusal — it is five ordinary characters, and
#      reading it as anything else would be a wrong tree.
#   4. A `-` AT EITHER END IS A MEMBER. `[a-]` matched `a` and `-`, so a range
#      needs a character on both sides of it.
#   5. `[[:alpha:]]` WORKS, and its inner `]` is not the close either — the
#      scan has to know about `[:…:]` or it ends one character early.
#   6. BASH PRINTS EVERY ONE OF THEM VERBATIM, so the second gate has no
#      opinion about any of the above. This is a construction-only build, and
#      the only oracle for it is matching — which is what section 1 does,
#      against real files.
#
# ⚠ And `[` alone is the TEST BUILTIN, whose `]` is a separate word. Reading
# every `[` as a bracket expression would throw the commonest conditional in the
# corpus away for a construct that is not there.
set -euo pipefail

scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT
cd "$scratch"
# One file per character the shapes below reach for, plus the awkward names.
: > a; : > b; : > z; : > 0; : > 9; : > -; : > ']'; : > '^'; : > '!'; : > 'ab'

section() { printf '\n=== %s\n' "$1"; }

# What does this pattern expand to? `matched` when it names a set, `LITERAL`
# when bash leaves it alone — which is how a non-glob announces itself, since an
# unmatched pattern expands to its own text.
expand() {
  local pattern=$1 out
  # shellcheck disable=SC2206
  out=($pattern)
  if [[ ${#out[@]} -eq 1 && ${out[0]} == "$pattern" ]]; then
    printf 'LITERAL (expanded to itself)'
  else
    printf 'matched: %s' "${out[*]}"
  fi
}

section "1 · which texts name a set"
for p in '[ab]' '[a-z]' '[!a]' '[^a]' '[0-9]' '[]a]' '[a-]' '[[:alpha:]]' \
         '[abc' 'a]' '[]' '[!]' '[ab]*' 'a[b]' '[a][b]'; do
  printf -- '%-14s → ' "$p"
  expand "$p"
  printf '\n'
done

section "2 · is ^ really a negation, or a member?"
# ⚠ POSIX spells it `!`; whether `^` negates or matches a literal caret decides
# whether the two are one tree.
printf '[^a] against a and ^: '; expand '[^a]'; printf '\n'
printf '[!a] against a and ^: '; expand '[!a]'; printf '\n'

section "3 · what bash PRINTS, which is nothing to go on"
render() {
  unset -f __p__ 2>/dev/null
  eval "__p__() {
$1
}" 2>/dev/null || { printf '<<REFUSED>>'; return 1; }
  declare -f __p__ | sed '1,2d;$d;s/^    //'
}
for s in 'echo [ab]' 'echo [!a-z]' 'echo [^a-z]' 'echo [[:alpha:]]' \
         'echo "[ab]"' 'echo [ -f x ]' 'ls .[!.]*' 'echo a[b]c'; do
  printf -- '--- in  %s\n    out ' "$s"
  if ! once=$(render "$s"); then printf '%s\n' "$once"; continue; fi
  printf '%s' "$once"
  if ! twice=$(render "$once"); then printf ' | TWICE REFUSED\n'; continue; fi
  [[ $once == "$twice" ]] && printf ' | fixpoint\n' || printf ' | DIFFERS %s\n' "$twice"
done

section "4 · the test builtin, which shares the character"
# ⚠ `[ -f x ]` is a COMMAND named `[`, and its `]` is a separate word. Reading
# every `[` as a bracket expression would throw the commonest conditional in the
# corpus away for a construct that is not there.
printf 'type -t [ → '; type -t '['
printf '[ is a word on its own: '; expand '['
