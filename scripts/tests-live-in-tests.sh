#!/usr/bin/env bash
# Every test in this workspace lives in `tests/`, and each crate's Cargo.toml says
# so by declaring `test = false` on its lib and bins. That declaration is a claim
# cargo does not check: a `#[test]` written under `src/` is simply never run, and
# nothing says it was skipped. This is what makes the claim fail instead.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

# Anchored to the start of a line, because the attribute is also written inside
# doc comments that talk about it — `reader/src/sniff.rs` is one.
found=$(git ls-files '*.rs' | grep -E '(^|/)src/' \
  | xargs grep -ln '^[[:space:]]*#\[\(tokio::\)\?test\]' || true)

if [ -n "$found" ]; then
  echo "A #[test] under src/ is never run — these crates declare test = false:"
  echo "$found" | sed 's/^/  /'
  echo "Move it to the crate's tests/suite/, or drop test = false for that target."
  exit 1
fi
