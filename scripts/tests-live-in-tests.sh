#!/usr/bin/env bash
# Every test in this workspace lives in `tests/`, so the gate's `tests` row runs
# the integration targets only — see the note on it in gate.dhall. That leaves a
# gap cargo will not close: a `#[test]` written under `src/` is simply never run,
# and nothing says it was skipped. This is what makes it fail instead.
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
