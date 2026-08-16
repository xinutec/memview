#!/usr/bin/env bash
# What does bash's own printer do to a parse? — the second gate, measured.
#
#   nix develop -c bash reader/probes/bash-printer.sh
#
# `declare -f` on a wrapped command makes bash render its parse as text. That is
# the only way to see bash's tree without running the command, so it is the
# independent oracle docs/execution-model.md specifies. This probe is what the
# claims in that section are made of; re-run it against a new bash before
# trusting any of them.
#
# Measured 2026-08-16, bash 5.3.15. Four findings, in the order they change the
# design:
#
#   1. It is a FIXPOINT. Every shape below prints its own print unchanged.
#   2. It is VERBATIM ON WORDS. `a`, `'a'`, `"a"`, `ec'h'o`, `"a""b"` and
#      `${x:-y}` all come back as written — only `$'…'`, `$"…"` and a
#      backslash-newline are resolved. So the gate cannot be a text comparison
#      against a canonical printer, and it says NOTHING about a word-internal
#      blind spot: an unimplemented `${x:-y}` absorbed into a literal is printed
#      back and absorbed again. Compare TREES, and defend words by construction.
#   3. It NORMALISES AND DESUGARS STRUCTURE, which is where its power is:
#      `|&` becomes `2>&1 |`, `! time a | b` becomes `time ! a | b`, a function
#      body written `( … )` is wrapped in a brace group, and a compound is laid
#      out with `do` on its own line. That last one already fires against the
#      current flat grammar, which reads the laid-out form as one command more.
#   4. It DELETES COMMENTS, so they are excluded from the comparison.
#
# ⚠ **And it executes.** See the last section: the function wrapper holds only
# because `eval` parses its whole argument before running any of it, and a
# balanced payload defeats that. The corpus is shell history and carries such
# text by accident, so the gate runs sandboxed.
# Every failing command here is a shape bash REFUSED, and each is caught by an
# `if !` or a `||` — where `-e` is suspended — so strict mode costs nothing.
set -euo pipefail

# The body of `f`, as bash prints it: drop the `f ()` and `{` lines and the
# closing `}`, then undo the eight-space indent bash puts on a top-level body.
# Nested lines keep whatever indentation bash gave them, which is the point.
render() {
  unset -f __p__ 2>/dev/null
  eval "__p__() {
$1
}" 2>/dev/null || { printf '<<REFUSED>>'; return 1; }
  declare -f __p__ | sed '1,2d;$d;s/^        //'
}

section() { printf '\n=== %s\n' "$1"; }

section "1 · fixpoint, and what changes on the way"
# Shapes chosen because each one asks a different question of the design: the
# quoting collapse, the pipeline flags, the compounds, heredocs, comments.
shapes=(
  "echo a"            "echo 'a'"          'echo "a"'          "ec'h'o a"
  'echo "a""b"'       'echo   a    b'      'echo ${x:-y}'      'echo "${a[1]}"'
  "echo \$'\\x41'"    'echo $"hello"'      'echo a\
b'
  "time ./x.sh"       "'time' ./x.sh"      'time -p ls | wc -l'
  '! grep -q x f'     '! time a | b'       'time { a; b; }'
  'ls |& cat'         'cat < in > out 2>&1'  'cat 2>&-'        'echo x >| out'
  'a; b'              'a & b'              'a && b || c'
  'for f in *.log; do echo "$f"; done'
  'while read -r l; do echo "$l"; done < f'
  'if ! a; then b; else c; fi'
  'case $x in a) b;;& c) d;; esac'
  'f() { echo hi; }'  'a() ( echo sub )'   '(cd /tmp && ls)'
  '{ echo a; echo b; } > out'
  'cat <<EOF
line $x
EOF'
  'echo "$(cat <<X
inner
X
)"'
  'echo a # trailing'
)
for s in "${shapes[@]}"; do
  printf -- '--- in\n%s\n--- out\n' "$s"
  if ! once=$(render "$s"); then printf '%s\n' "$once"; continue; fi
  printf '%s\n' "$once"
  if ! twice=$(render "$once"); then printf 'TWICE: <<REFUSED>>\n'; continue; fi
  [[ $once == "$twice" ]] && printf 'TWICE: fixpoint\n' \
                          || printf 'TWICE: DIFFERS\n%s\n' "$twice"
done

section "2 · the wrapper does not contain the command"
# ⚠ Demonstrated, because the safety claim in the doc is worth more as an
# observation than as an argument. The payload closes the function, runs, and
# reopens a group for the wrapper's own trailing brace to close — so eval's
# whole-string parse succeeds and there is nothing left to refuse.
#
# It writes one file inside a scratch directory this script removes. Nothing
# here needs a real corpus command to make the point.
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT
eval "__esc__() {
echo a; }; : > '$scratch/ESCAPED'; { echo b
}" >/dev/null 2>&1
if [[ -e $scratch/ESCAPED ]]; then
  echo "⚠ ESCAPED — a balanced payload runs. The gate must be sandboxed."
else
  echo "contained — re-read the doc's safety note before relying on this"
fi
