#!/usr/bin/env bash
# What is `$'…'` to bash? — measured, not remembered.
#
#   nix develop -c bash reader/probes/ansi-quote.sh
#
# The question is whether it is a construct at all. If bash resolves the escapes
# at parse time then `$'\x41'` and `'A'` are the same command, and the tree has
# to say so — a node preserving the spelling would make one command two trees
# and the second gate would object.
#
# Measured 2026-08-17, bash 5.3.15. Five findings, in the order they change the
# design:
#
#   1. IT IS A SPELLING OF A LITERAL, not an expansion. Every shape below comes
#      back as an ordinary single-quoted string with the escape RESOLVED:
#      `$'\x41'` → `'A'`, `$'\101'` → `'A'`, `$'a\tb'` → `'a<TAB>b'`,
#      `$'a\nb'` → a string holding a real newline. So it resolves here too, and
#      the segment it produces is a `Literal`.
#   2. WHICH MEANS THE SECOND GATE CAN CHECK THE DECODING. Everywhere else
#      inside a word bash is verbatim and has no opinion; here it does the
#      decoding itself and prints the result, so a wrong escape is a difference
#      between two trees rather than a mistake both sides share. This is the one
#      word-internal construct that is not construction-only.
#   3. AN UNKNOWN ESCAPE KEEPS ITS BACKSLASH. `$'\z'` is the two characters
#      `\z`, not `z`.
#   4. `\u` AND `\U` ARE NOT RESOLVED BY THIS BUILD. `$'é'` comes back as
#      the six characters `é` — parsed and re-spelled with the hex
#      uppercased, rather than turned into a character. The parser refuses those
#      two escapes rather than guess which of the two behaviours to match.
#   5. IT IS A SEGMENT, SO IT GLUES, AND IT QUOTES THE WORD. `x$'a'y` is one
#      word and `$'a b'` is one argument.
#   6. A NUL EXPANDS TO NOTHING AT ALL — section 2 below shows `\0` producing no
#      bytes. So the tree may not hold one: printing the character back would
#      write a byte bash then drops. Refused, and the second gate found the
#      same two corpus commands (`$'\x00'`, `$'\x00GITCRYPT'`) on its own before
#      the refusal was in.
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
# ⚠ Piped through `cat -v`, or a resolved control character is invisible in the
# output and the finding cannot be read off it.
shapes=(
  "echo \$'\\x41'"      # hex
  "echo \$'\\101'"      # octal
  "echo \$'a\\tb'"      # the named escapes
  "echo \$'a\\nb'"
  "echo \$'\\e[1m'"
  "echo \$'\\cA'"       # a control character by name
  "echo \$'\\\\'"       # an escaped backslash
  "echo \$'it\\'s'"     # an escaped quote, which does NOT close it
  "echo \$'\\z'"        # an escape bash does not know
  "echo \$'\\u00e9'"    # the one it does not resolve
  "echo \$''"           # empty
  "echo x\$'a'y"        # glued into a word
  "echo \$'a b'"        # one argument, not two
  "IFS=\$'\\n'"         # the shape the corpus actually holds
  "echo \"\$'a'\""      # inside double quotes it is a dollar and a quote
)
for s in "${shapes[@]}"; do
  printf -- '--- in  %s\n    out ' "$s"
  if ! once=$(render "$s"); then printf '%s\n' "$once"; continue; fi
  printf '%s' "$once" | cat -v
  if ! twice=$(render "$once"); then printf ' | TWICE REFUSED\n'; continue; fi
  if [[ $once == "$twice" ]]; then
    printf ' | fixpoint\n'
  else
    printf ' | DIFFERS '
    printf '%s' "$twice" | cat -v
    printf '\n'
  fi
done

section "2 · what the escapes actually produce, byte for byte"
# The decoding the parser has to reproduce, read off bash rather than off a
# manual page.
for e in '\x41' '\101' '\t' '\n' '\r' '\a' '\b' '\e' '\E' '\f' '\v' '\0' '\cA' '\cz' '\z' '\?'; do
  printf "%-6s → " "$e"
  eval "printf '%s' \$'$e'" | od -An -tx1 | tr -s ' '
done
