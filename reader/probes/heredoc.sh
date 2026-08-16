#!/usr/bin/env bash
# What is a heredoc, to bash? — measured, not remembered.
#
#   nix develop -c bash reader/probes/heredoc.sh
#
# `declare -f` renders bash's own parse as text (see bash-printer.sh for why
# that is the second gate). This probe asks it the questions a heredoc node has
# to answer, because a heredoc is the first construct whose operand is not on
# the line that opens it.
#
# Measured 2026-08-16, bash 5.3.15. Six findings, in the order they change the
# design:
#
#   1. THE DELIMITER'S SPELLING IS NOT KEPT. `<<'EOF'`, `<<"EOF"`, `<<\EOF` and
#      `<<E"O"F` all print back as `<<'EOF'`, so bash keeps the text and one bit
#      — was it quoted — and forgets the rest. The tree does the same.
#   2. THAT BIT CHANGES THE BODY, not just what will expand later. A
#      backslash-newline inside an UNQUOTED body is joined at parse time (`a\⏎b`
#      is stored as `ab`); inside a quoted one it survives. And the join is not a
#      text replacement — `a\\⏎b` stays two lines, because the escaped backslash
#      protects the newline.
#   3. `<<-` STRIPS AT PARSE TIME. The body comes back unindented with the `-`
#      still on the operator, so the flag is not recoverable from the body and
#      both have to be stored.
#   4. THE BODY FOLLOWS THE LOGICAL LINE. A newline inside a quoted word, or
#      after a backslash, does not start it — so the body cannot be found by
#      scanning ahead for the next `\n`.
#   5. NOTHING PAIRS AN OPENER WITH A BODY BUT ORDER. `cat <<A <<A` is legal and
#      its two bodies differ.
#   6. AN UNTERMINATED HEREDOC IS ACCEPTED, and the rest of the input becomes
#      the body, with `warning: here-document at line N delimited by
#      end-of-file` on stderr. So `bash -n`'s exit code cannot adjudicate it,
#      and `declare -f` cannot render it at all — the runaway body swallows the
#      wrapper's closing brace. The parser reads it the way bash does; the
#      round-trip law covers it, gate 2 excludes it as it excludes comments, and
#      `bash_warns_of_a_runaway_heredoc` is what tells that exclusion apart from
#      a command bash refused for a real reason.
set -euo pipefail

render() {
  unset -f __p__ 2>/dev/null
  eval "__p__() {
$1
}" 2>/dev/null || { printf '<<REFUSED>>'; return 1; }
  declare -f __p__ | sed '1,2d;$d;s/^        //'
}

section() { printf '\n=== %s\n' "$1"; }

# A literal tab, for the `<<-` shapes. Written this way so the file survives an
# editor that trims or expands whitespace.
T=$'\t'

section "1 · what comes back, and is it a fixpoint"
shapes=(
  # the plain form, and whether the body is verbatim
  'cat <<EOF
plain
EOF'
  'cat <<EOF
$x `date` ${y:-z} \$lit
EOF'
  # an empty body, and a body that is only a newline
  'cat <<EOF
EOF'
  # does the DELIMITER spelling survive? these three suppress expansion...
  "cat <<'EOF'
\$x stays
EOF"
  'cat <<"EOF"
$x stays
EOF'
  'cat <<\EOF
$x stays
EOF'
  # ...and this one is quoted only in part, which still suppresses
  'cat <<E"O"F
$x stays
EOF'
  # <<- strips leading TABS from body and terminator
  "cat <<-EOF
${T}indented body
${T}EOF"
  "cat <<-'EOF'
${T}both features at once
${T}EOF"
  # spaces are NOT stripped by <<-
  "cat <<-EOF
${T}  tab then spaces
${T}EOF"
  # several open at once — which body belongs to which delimiter?
  'cat <<A <<B
first
A
second
B'
  # a heredoc beside ordinary redirects, and the order bash prints them in
  'cat <<EOF > out
body
EOF'
  'cat > out <<EOF
body
EOF'
  'cat <<EOF 2>&1 | wc -l
body
EOF'
  # an explicit descriptor
  'cat 3<<EOF
body
EOF'
  # the body is not scanned for the delimiter as a substring
  'cat <<EOF
EOFEOF and EOF x
EOF'
  # a here-STRING, for contrast: same operator family, operand on the line
  'cat <<< "a string"'
  'cat <<<$x'
  # the second command on the line starts AFTER the body
  'cat <<EOF; echo after
body
EOF'
  # a heredoc inside a compound, where indentation is bash'"'"'s to choose
  'if true; then cat <<EOF
body
EOF
fi'
)
for s in "${shapes[@]}"; do
  printf -- '--- in\n%s\n--- out\n' "$s"
  if ! once=$(render "$s"); then printf '%s\n' "$once"; continue; fi
  printf '%s\n' "$once"
  if ! twice=$(render "$once"); then printf 'TWICE: <<REFUSED>>\n'; continue; fi
  [[ $once == "$twice" ]] && printf 'TWICE: fixpoint\n' \
                          || printf 'TWICE: DIFFERS\n%s\n' "$twice"
done

section "2 · what does bash REFUSE"
# Each of these is a claim about the input that `bash -n` adjudicates, which is
# how a refusal earns the right to be called the input's fault rather than ours.
bad=(
  'cat <<'                      # no delimiter
  'cat <<EOF'                   # delimiter never appears
  'cat <<EOF
body'                           # same, with a body
  'cat <<EOF
body
 EOF'                           # terminator indented, plain form
  "cat <<-EOF
body
${T} EOF"                       # tab then space before the terminator
  'cat <<EOF
body
EOF '                           # trailing space after the terminator
)
for s in "${bad[@]}"; do
  printf -- '--- in\n%s\n--- ' "$s"
  if printf '%s\n' "$s" | bash -n 2>/dev/null; then
    printf 'bash -n ACCEPTS\n'
  else
    printf 'bash -n refuses\n'
  fi
done
