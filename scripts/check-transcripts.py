#!/usr/bin/env nix-shell
#!nix-shell -i python3 -p python3
"""Decide whether a transcript is structurally sound, and say why when it is not.

The readers in this repo are deliberately lenient -- `serde_json::from_slice(..)
.ok()?` skips a line it cannot parse and carries on -- so a transcript that has
lost a message renders exactly like one that has not. That leniency is right for
a viewer and useless as a guarantee. This is the strict counterpart: it exists to
be the thing that says no.

Every rule below was derived by measuring the whole corpus, not by reading a
schema, because there is no schema. Where the measurement contradicted an
assumption the measurement won -- see LINE TYPES, which has sixteen entries
because a survey that found fifteen missed `pr-link` entirely.

Exit status is 0 only if every file passes every rule.

    ./check-transcripts.py ~/.claude/projects            # whole corpus
    ./check-transcripts.py path/to/one.jsonl             # a single file
    ./check-transcripts.py --quiet DIR                   # totals only
"""

import argparse
import collections
import json
import pathlib
import re
import sys

# A line either belongs to the conversation or describes it. The distinction is
# not cosmetic: it decides whether the line is expected to carry identity at all.
# Measured over 703k lines with no exception in either direction.
CONVERSATION_TYPES = {"assistant", "user", "attachment", "system"}

METADATA_TYPES = {
    "last-prompt",
    "permission-mode",
    "bridge-session",
    "mode",
    "queue-operation",
    "ai-title",
    "agent-name",
    "custom-title",
    "file-history-snapshot",
    "file-history-delta",
    "pr-link",
    "frame-link",
}

KNOWN_TYPES = CONVERSATION_TYPES | METADATA_TYPES

# Only `user` and `system` were ever observed starting a chain (2,224 and 1,036
# explicit nulls). An `assistant` root does not occur once in 536,429 assistant
# lines, so one appearing means something severed the chain above it.
ROOTABLE_TYPES = {"user", "system"}

UUID_RE = re.compile(r"\A[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\Z")


def is_transcript(path):
    """Whether this file is a conversation, as opposed to something else `.jsonl`.

    ⚠ Stricter than `reader::transcript::is_transcript`, on purpose. That one
    tests the extension alone, which is the right rule for a viewer that wants
    every conversation in the tree and can afford to shrug at anything odd. It is
    the wrong rule here: a session's sidecar directory holds
    `subagents/workflows/wf_*/journal.jsonl`, a different format with its own
    `started`/`result` line types and no uuid anywhere. Feed those to a strict
    checker and it reports 1,052 violations that are not defects -- they are a
    different file being read as the wrong thing.

    A transcript is named for the session it records, so the name is the test.
    """
    return path.suffix == ".jsonl" and UUID_RE.match(path.stem) is not None


class Report:
    """Violations for one file, grouped by rule so a systemic fault reads as one
    finding rather than as ten thousand."""

    def __init__(self, path):
        self.path = path
        self.failures = collections.defaultdict(list)
        self.lines = 0

    def fail(self, rule, detail):
        self.failures[rule].append(detail)

    @property
    def ok(self):
        return not self.failures

    def render(self, examples=3):
        out = [f"FAIL {self.path}  ({self.lines} lines)"]
        for rule, hits in sorted(self.failures.items()):
            out.append(f"  {rule}: {len(hits)}")
            for detail in hits[:examples]:
                out.append(f"      {detail}")
            if len(hits) > examples:
                out.append(f"      ... and {len(hits) - examples} more")
        return "\n".join(out)


def check_file(path):
    report = Report(path)
    uuids = set()
    parent_of = {}
    seen = {}          # uuid -> (type, parentUuid) of its first occurrence
    edges = []         # (uuid, parentUuid) for every line that has both

    with open(path, "rb") as handle:
        for lineno, raw in enumerate(handle, 1):
            stripped = raw.strip()
            if not stripped:
                continue
            report.lines += 1

            try:
                node = json.loads(stripped)
            except json.JSONDecodeError as exc:
                report.fail("unparseable", f"line {lineno}: {exc}")
                continue
            if not isinstance(node, dict):
                report.fail("not-an-object", f"line {lineno}")
                continue

            kind = node.get("type")
            if kind not in KNOWN_TYPES:
                report.fail("unknown-type", f"line {lineno}: {kind!r}")
                continue

            uuid = node.get("uuid")
            has_parent_field = "parentUuid" in node
            parent = node.get("parentUuid")

            if kind in CONVERSATION_TYPES:
                # Identity is mandatory here. Without it the line cannot be
                # placed in the conversation, which is the only reason to keep it.
                if not uuid:
                    report.fail("missing-uuid", f"line {lineno}: type={kind}")
                elif not UUID_RE.match(uuid):
                    report.fail("malformed-uuid", f"line {lineno}: {uuid!r}")
                if not has_parent_field:
                    report.fail("missing-parent-field", f"line {lineno}: type={kind}")

                if parent is None and has_parent_field and kind not in ROOTABLE_TYPES:
                    report.fail(
                        "unrootable-type-at-root",
                        f"line {lineno}: {kind} has no parent; only "
                        f"{sorted(ROOTABLE_TYPES)} were ever observed as roots",
                    )

                if uuid:
                    uuids.add(uuid)
                    prior = seen.get(uuid)
                    if prior is None:
                        seen[uuid] = kind
                    elif prior != kind:
                        # A uuid is re-emitted constantly -- 53,376 distinct
                        # uuids over 309,290 repeat events -- so repetition
                        # itself proves nothing. What never happens is a uuid
                        # changing what KIND of thing it is: 0 occurrences. A
                        # line claiming otherwise is a genuine contradiction.
                        #
                        # Its `parentUuid` is a different matter and is NOT
                        # checked: 432 repeats legitimately move a node to a new
                        # parent, which is how an edited or re-run turn is
                        # recorded. Requiring parent agreement would fail files
                        # that are merely old.
                        report.fail(
                            "uuid-type-change",
                            f"line {lineno}: {uuid} was {prior!r}, now {kind!r}",
                        )
                    if isinstance(parent, str):
                        edges.append((uuid, parent))
                        parent_of.setdefault(uuid, parent)
            else:
                # Metadata describes the conversation from outside it and never
                # carries identity. A metadata line with a uuid means the writer
                # is doing something this checker has not seen.
                if uuid is not None:
                    report.fail("metadata-with-uuid", f"line {lineno}: type={kind}")
                if has_parent_field:
                    report.fail("metadata-with-parent", f"line {lineno}: type={kind}")

            if has_parent_field and parent is not None and not isinstance(parent, str):
                report.fail("non-string-parent", f"line {lineno}: {parent!r}")

    # Resolution is checked only after the whole file is read: a parent may be
    # written after its child, and ordering is not something to assume.
    for uuid, parent in edges:
        if parent not in uuids:
            report.fail("dangling-parent", f"{uuid} -> {parent} (no such uuid)")

    # Cycles. Untested before now, so measured rather than assumed. Iterative on
    # purpose -- these chains run to hundreds of thousands of nodes and recursion
    # would hit the interpreter limit long before it found anything.
    colour = {}
    for start in parent_of:
        if colour.get(start):
            continue
        path_nodes = []
        node = start
        while node is not None and colour.get(node) is None:
            colour[node] = "grey"
            path_nodes.append(node)
            node = parent_of.get(node)
        if node is not None and colour.get(node) == "grey":
            loop = path_nodes[path_nodes.index(node):]
            report.fail("cycle", f"{len(loop)} nodes, starting {loop[0]}")
        for seen_node in path_nodes:
            colour[seen_node] = "black"

    return report


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("target", help="a .jsonl transcript or a directory of them")
    ap.add_argument("--quiet", action="store_true", help="totals only")
    ap.add_argument("--examples", type=int, default=3)
    args = ap.parse_args()

    root = pathlib.Path(args.target).expanduser()
    if root.is_dir():
        files = sorted(p for p in root.glob("**/*.jsonl") if is_transcript(p))
    else:
        files = [root]
    if not files:
        sys.exit(f"no .jsonl files under {root}")

    bad = 0
    totals = collections.Counter()
    lines = 0
    for path in files:
        report = check_file(path)
        lines += report.lines
        if report.ok:
            continue
        bad += 1
        for rule, hits in report.failures.items():
            totals[rule] += len(hits)
        if not args.quiet:
            print(report.render(args.examples))

    print(f"\n{len(files)} file(s), {lines} lines, {bad} failing")
    if totals:
        print("violations by rule:")
        for rule, count in totals.most_common():
            print(f"  {count:8d}  {rule}")
        sys.exit(1)
    print("all invariants hold")


if __name__ == "__main__":
    main()
