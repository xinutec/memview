#!/usr/bin/env bash
# What is `case … esac` to bash? — measured, not remembered.
#
#   nix develop -c bash reader/probes/case.sh
#
# It is the top of the refusal queue: 134 corpus commands need it and nothing
# else. `case` is the first construct whose interior is not a command list — an
# arm is a list of PATTERNS, which is a third grammar beside words and
# arithmetic, and the terminator between arms carries meaning of its own.
#
# Measured 2026-08-17, bash 5.3.15. Seven findings, in the order they change the
# design:
#
#   1. A PATTERN IS PRINTED VERBATIM, exactly as a word is. `"a b")`, `'lit')`,
#      `a\ b)`, `$y)`, `${z:-w})` and `$(cmd))` all come back as written, so the
#      second gate has NO OPINION about what is inside a pattern — the same
#      blind spot it has about a word, and the same answer: build the pattern as
#      a word, by construction, and never absorb one into a literal.
#   2. A LEADING `(` IS NOT KEPT. `(a)` and `(c|d)` come back as `a)` and
#      `c | d)`, so the tree must not record it either — recording it would be a
#      distinction bash collapsed, and gate 2 could never object.
#   3. THREE TERMINATORS, ALL KEPT AND ALL DIFFERENT. Section 3 runs them: `;;`
#      stops, `;&` runs the next arm's body WITHOUT testing its pattern, and
#      `;;&` goes on testing. They are distinct programs, so the arm holds
#      which one it was.
#   4. THE LAST ARM'S TERMINATOR IS SUPPLIED. `case $x in a) esac` — an empty
#      body and no `;;` — comes back with `;;` written in. And `case $x in esac`
#      is a legal case with NO ARMS AT ALL.
#   5. AN EMPTY BODY IS LEGAL, and the corpus writes it for "match this and do
#      nothing". `a);;`, `a) ;;` and `a) ;;esac` all render.
#   6. A PATTERN IS NOT A COMMAND POSITION, so `in)` and `do)` are ordinary
#      patterns — but `esac)` is a SYNTAX ERROR, because the `esac` is taken as
#      the terminator before the `)` is reached. An unquoted `)` ends a pattern
#      wherever it is; a quoted or escaped one (`'a)b'`, `"a)b"`, `a\)b`) does
#      not, and neither does the one closing a `$(f a)` inside one.
#   7. AN ARM'S BODY MAY END IN `&`. `a) b & ;;` renders, unlike `{ a & ; }`
#      which is a syntax error — so the printer needs no separator before `;;`.
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
  # the plain form
  'case $x in a) b;; esac'
  # several arms, and a last one with no terminator
  'case $x in a) b;; c) d;; esac'
  'case $x in a) b;; c) d esac'
  # the three terminators: stop, fall through, keep matching
  'case $x in a) b;; c) d;& e) f;;& g) h;; esac'
  # an EMPTY body, which the corpus writes for "do nothing"
  'case $x in a) ;; *) b;; esac'
  'case $x in a) esac'
  # several patterns per arm
  'case $x in a|b|c) d;; esac'
  # a leading `(`, which is legal and which POSIX prefers
  'case $x in (a) b;; (c|d) e;; esac'
  # what a PATTERN may be: globs, brackets, quoting, expansions
  'case $x in *.txt) a;; ?) b;; [abc]) c;; esac'
  'case $x in "a b") c;; '"'"'lit'"'"') d;; esac'
  'case $x in $y) a;; ${z:-w}) b;; $(cmd)) c;; esac'
  'case $x in a\ b) c;; esac'
  # a `)` that does NOT end the pattern, four ways
  "case \$x in 'a)b') c;; esac"
  'case $x in "a)b") c;; esac'
  'case $x in a\)b) c;; esac'
  'case $x in $(f a) ) c;; esac'
  # a keyword as a pattern, and `esac` itself
  'case $x in in) a;; do) b;; esac'
  'case $x in esac'
  # an empty body written three ways, and a body ending in `&`
  'case $x in a);; esac'
  'case $x in a) ;;esac'
  'case $x in a) b & ;; esac'
  # a fall-through on the LAST arm, where it has nothing to fall to
  'case $x in a) b;& esac'
  # the SUBJECT: is it an ordinary word?
  'case "$x" in a) b;; esac'
  'case ${x:-d} in a) b;; esac'
  'case $(f) in a) b;; esac'
  # an arm body holding more than one command, and a compound
  'case $x in a) b; c;; esac'
  'case $x in a) if y; then z; fi;; esac'
  'case $x in a) for f in 1; do g; done;; esac'
  # nested, and a case inside an arm
  'case $x in a) case $y in b) c;; esac;; esac'
  # redirection on the whole thing, and in an arm
  'case $x in a) b;; esac > out'
  'case $x in a) b > out;; esac'
  # a heredoc in an arm, whose body has to follow the line
  'case $x in a) cat <<X
body
X
;; esac'
  # comments
  'case $x in # which
  a) b;; esac'
  # written across lines, which is how a script spells it
  'case $x in
  a)
    b
    ;;
  *)
    c
    ;;
esac'
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
bad=(
  'case $x in a) b;;'            # no esac
  'case in a) b;; esac'          # no subject
  'case $x a) b;; esac'          # no `in`
  'case $x in ) b;; esac'        # an empty pattern
  'case $x in a b) c;; esac'     # two words as one pattern
  'case $x in a|) b;; esac'      # an empty pattern after a `|`
  'case $x in esac) a;; esac'    # `esac` as a pattern
)
for s in "${bad[@]}"; do
  printf -- '--- in\n%s\n--- ' "$s"
  if printf '%s\n' "$s" | bash -n 2>/dev/null; then
    printf 'bash -n ACCEPTS\n'
  else
    printf 'bash -n refuses\n'
  fi
done

section "3 · what the terminators DO, run for real"
# ⚠ Three of them, and the difference is only visible by running: `;;` stops,
# `;&` runs the next arm's body without testing it, `;;&` goes on testing.
for op in ';;' ';&' ';;&'; do
  printf '%-4s → ' "$op"
  eval "case ab in
    a*) printf 'first ' $op
    *b) printf 'second ' ;;
  esac"
  printf '\n'
done
