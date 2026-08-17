#!/usr/bin/env bash
# What does a backslash mean inside quotes? — measured, not remembered.
#
#   nix develop -c bash reader/probes/quoting.sh
#
# Asked because `reader/src/shell.rs` answered it wrongly for a year and nothing
# in the repository could see it. That module resolves quoting by hand, and its
# double-quote branch dropped EVERY backslash it met — so `grep -E "\s+"` was
# read as `grep -E "s+"`, and a `bash -c "… tr -d '\r' …"` payload came out
# meaning a letter r. The syntax tree disagreed on 21,481 of the corpus's 134,622
# distinct commands, which is how the question got asked at all
# (`cargo run -p reader --bin projection`).
#
# Measured 2026-08-17, bash 5.3.15. Four findings:
#
#   1. INSIDE DOUBLE QUOTES A BACKSLASH ESCAPES FIVE THINGS AND NOTHING ELSE:
#      `$`, a backquote, `"`, `\`, and a newline. Before any other character it
#      is an ordinary backslash and stays in the value.
#   2. THE NEWLINE CASE IS NOT AN ESCAPE. `"a\<newline>b"` is `ab`: both
#      characters are removed, which is a line continuation, not a value.
#   3. INSIDE SINGLE QUOTES NOTHING IS AN ESCAPE, INCLUDING A BACKSLASH.
#      `'a\rb'` is five characters and there is no way to write a `'` in there.
#   4. OUTSIDE QUOTES A BACKSLASH ESCAPES WHATEVER FOLLOWS IT, which is the one
#      case the flat reader already had right.
#
# Printed through `printf %s` rather than `echo`, because `echo` in some shells
# interprets escapes itself and would be measuring the wrong program.

set -euo pipefail

say() { printf '%-28s [%s]\n' "$1" "$2"; }

echo "--- 1. double quotes: what is an escape ---"
say 'value of "\$x"'      "$(printf '%s' "\$x")"
say 'value of "\`x"'      "$(printf '%s' "\`x")"
say 'value of "\"x"'      "$(printf '%s' "\"x")"
say 'value of "\\x"'      "$(printf '%s' "\\x")"
echo "--- and what is not ---"
say 'value of "\.lpass"'  "$(printf '%s' "\.lpass")"
say 'value of "lpass \["' "$(printf '%s' "lpass \[")"
say 'value of "\s+"'      "$(printf '%s' "\s+")"
say 'value of "\r"'       "$(printf '%s' "\r")"
say 'value of "\z"'       "$(printf '%s' "\z")"

echo "--- 2. a backslash-newline inside double quotes is removed ---"
say 'value of "a\<nl>b"' "$(printf '%s' "a\
b")"

echo "--- 3. single quotes escape nothing ---"
say "value of 'a\\rb'"   "$(printf '%s' 'a\rb')"
say "value of 'a\\\\b'"  "$(printf '%s' 'a\\b')"

echo "--- 4. unquoted, a backslash escapes anything ---"
say 'value of \.lpass'   "$(printf '%s' \.lpass)"
say 'value of a\ b'      "$(printf '%s' a\ b)"

echo "--- 5. and the parser agrees with all of the above ---"
# `declare -f` prints bash's own parse back. Every value above is a LITERAL to
# it, so what comes out is the one spelling bash chooses for that literal — which
# is the check the second gate makes on every corpus command.
q() { printf '%s' "\.lpass" "lpass \[" 'a\rb' \.lpass; }
declare -f q
