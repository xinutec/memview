#!/usr/bin/env bash
# What is `<( … )` to bash? — measured, not remembered.
#
#   nix develop -c bash reader/probes/process-substitution.sh
#
# It is the top of the refusal queue: 212 corpus commands need it and nothing
# else. This probe asks the questions its node has to answer, because it is the
# first construct that is a WORD whose value is a path bash invents — and the
# only one so far that can appear both as an argument and as a redirection
# target.
#
# Measured 2026-08-17, bash 5.3.15. Six findings, in the order they change the
# design:
#
#   1. THE INTERIOR IS NORMALISED, not verbatim. `<(a|b)` comes back as
#      `<(a | b)` and `<(c   d)` as `<(c d)`, exactly as `$( )` does and unlike
#      a backtick. So the second gate CAN see inside one, and the node is a real
#      recursion into the script reader rather than an opaque run of text.
#   2. IT IS A WORD, and it glues. `diff x<(a)` is one word — the path bash
#      invents concatenated onto `x` — and `x=<(a)` is an assignment whose value
#      is one. So it is a segment, like a parameter, not a command of its own.
#   3. AND IT IS ALSO A REDIRECTION TARGET. `cat < <(ls)`, `cat 3< <(a)` and
#      `echo hi >> >(cat)` all render, so the same segment has to be reachable
#      from a redirection's target word. The `while … done < <(ls)` idiom is
#      what the corpus actually holds.
#   4. THE SPACE DECIDES. `diff < (a) b` — a blank between the operator and the
#      paren — is a SYNTAX ERROR to bash, not a redirection to a subshell. So
#      `<` immediately followed by `(` is the whole test, and there is no
#      ambiguity to resolve.
#   5. A HEREDOC WORKS INSIDE ONE, printed across lines exactly as it is inside
#      `$( )` — see substitution-heredoc.sh, whose treatment carries over.
#   6. THE VALUE IS A PATH BASH INVENTS: `/dev/fd/63`, and a second one in the
#      same command gets `/dev/fd/62`. Nothing about it is determined by the
#      text, which is why the reader counts it as naming no file rather than
#      resolving it. An empty one, `<()`, is accepted.
set -euo pipefail

# The body of `f`, as bash prints it — see reader/probes/bash-printer.sh.
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
  # the two directions, as arguments
  'diff <(sort a) <(sort b)'
  'tee >(wc -l) >(md5sum)'
  # as a redirection TARGET, which is the `while read` idiom
  'while read -r l; do echo "$l"; done < <(ls)'
  'cat < <(ls)'
  'cat > >(wc -l)'
  # is the INTERIOR normalised, the way $( ) is, or verbatim like a backtick?
  'diff <(a|b) <(c   d)'
  'diff <(a; b) <(if x; then y; fi)'
  'diff <($(echo a)) <(`echo b`)'
  # nested one level down
  'diff <(diff <(a) <(b)) c'
  # a heredoc inside one — the shape that took a build of its own for $( )
  'diff <(cat <<X
body
X
) b'
  # does the word glue to what is beside it?
  'diff x<(a)'
  'echo "<(a)"'
  "echo '<(a)'"
  # in an assignment, and in a for list
  'x=<(a)'
  'for f in <(a); do echo "$f"; done'
  # a descriptor in front of it, and the operator on its own
  'cat 3< <(a)'
  # redirection to a process substitution, appended
  'echo hi >> >(cat)'
)
for s in "${shapes[@]}"; do
  printf -- '--- in\n%s\n--- out\n' "$s"
  if ! once=$(render "$s"); then printf '%s\n' "$once"; continue; fi
  printf '%s\n' "$once"
  if ! twice=$(render "$once"); then printf 'TWICE: <<REFUSED>>\n'; continue; fi
  [[ $once == "$twice" ]] && printf 'TWICE: fixpoint\n' \
                          || printf 'TWICE: DIFFERS\n%s\n' "$twice"
done

section "2 · what bash REFUSES"
# Each of these is a claim about the input that `bash -n` adjudicates.
bad=(
  '<(a'                 # never closed
  'diff < (a) b'        # a space makes it a redirection to a subshell
  'diff <()'            # empty
)
for s in "${bad[@]}"; do
  printf -- '--- in\n%s\n--- ' "$s"
  if printf '%s\n' "$s" | bash -n 2>/dev/null; then
    printf 'bash -n ACCEPTS\n'
  else
    printf 'bash -n refuses\n'
  fi
done

section "3 · what the word's VALUE is, run for real"
# ⚠ It is a path bash invents, so nothing about it is determined by the text —
# which is the whole reason the reader counts it as a named-nothing rather than
# resolving it.
printf 'argument: '; echo <(true)
printf 'two of them differ: '; echo <(true) <(true)
