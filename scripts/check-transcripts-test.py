#!/usr/bin/env nix-shell
#!nix-shell -i python3 -p python3
"""Ablation for the transcript checker: break one thing, prove it complains.

A checker that has only ever been run against healthy input has demonstrated
nothing. `reader/src/transcript.rs` carries the scar -- a regression test there
passed with the fix ablated, because it was really testing whichever entry
`read_dir` happened to return first. So every rule here is exercised twice: once
against a transcript that satisfies it, once against the same transcript with a
single field broken, and the pair must disagree.

Fixtures are built in a temp directory. Nothing here reads a real conversation:
the corpus is one person's, and a test that depends on it fails for everyone else
and leaks their paths into a public repo besides.

    ./check-transcripts-test.py
"""

import json
import pathlib
import subprocess
import sys
import tempfile

CHECKER = pathlib.Path(__file__).with_name("check-transcripts.py")
SESSION = "0a1b2c3d-4e5f-4a6b-8c9d-0e1f2a3b4c5d"

U1 = "11111111-1111-4111-8111-111111111111"
U2 = "22222222-2222-4222-8222-222222222222"
U3 = "33333333-3333-4333-8333-333333333333"
ABSENT = "99999999-9999-4999-8999-999999999999"


def healthy():
    """A minimal but structurally complete conversation.

    Deliberately includes a metadata line and a re-emitted uuid, so that the
    happy path covers the two shapes most likely to be mistaken for damage.
    """
    return [
        {"type": "user", "uuid": U1, "parentUuid": None, "timestamp": "t0"},
        {"type": "assistant", "uuid": U2, "parentUuid": U1, "timestamp": "t1"},
        {"type": "mode", "mode": "default"},
        {"type": "user", "uuid": U3, "parentUuid": U2, "timestamp": "t2"},
        # Lawful re-emission: same node, later moved onto a different parent.
        {"type": "user", "uuid": U3, "parentUuid": U1, "timestamp": "t3"},
    ]


def run(lines):
    with tempfile.TemporaryDirectory() as tmp:
        path = pathlib.Path(tmp) / f"{SESSION}.jsonl"
        with open(path, "w") as handle:
            for line in lines:
                handle.write(
                    line if isinstance(line, str) else json.dumps(line)
                )
                handle.write("\n")
        proc = subprocess.run(
            [sys.executable, str(CHECKER), str(path)],
            capture_output=True,
            text=True,
        )
        return proc.returncode, proc.stdout + proc.stderr


def mutate(index, **changes):
    lines = healthy()
    for key, value in changes.items():
        if value is KeyError:
            lines[index].pop(key, None)
        else:
            lines[index][key] = value
    return lines


CASES = [
    # (name, lines, rule expected in the output)
    ("dangling parent", mutate(1, parentUuid=ABSENT), "dangling-parent"),
    ("unparseable line", healthy()[:2] + ["{not json"], "unparseable"),
    ("unknown type", mutate(1, type="invented-type"), "unknown-type"),
    ("missing uuid", mutate(1, uuid=KeyError), "missing-uuid"),
    ("malformed uuid", mutate(1, uuid="not-a-uuid"), "malformed-uuid"),
    ("missing parent field", mutate(1, parentUuid=KeyError), "missing-parent-field"),
    ("assistant as root", mutate(1, parentUuid=None), "unrootable-type-at-root"),
    ("metadata carrying a uuid", mutate(2, uuid=U1), "metadata-with-uuid"),
    ("metadata carrying a parent", mutate(2, parentUuid=U1), "metadata-with-parent"),
    ("non-string parent", mutate(1, parentUuid=17), "non-string-parent"),
    (
        "uuid changes type",
        healthy() + [{"type": "assistant", "uuid": U3, "parentUuid": U2}],
        "uuid-type-change",
    ),
    (
        "cycle",
        [
            {"type": "user", "uuid": U1, "parentUuid": U3},
            {"type": "assistant", "uuid": U2, "parentUuid": U1},
            {"type": "user", "uuid": U3, "parentUuid": U2},
        ],
        "cycle",
    ),
]


def main():
    code, output = run(healthy())
    if code != 0:
        print("FAIL: the healthy fixture did not pass\n" + output)
        return 1
    print("ok   healthy fixture passes")

    failures = 0
    for name, lines, rule in CASES:
        code, output = run(lines)
        if code == 0:
            print(f"FAIL {name}: checker accepted it (expected {rule})")
            failures += 1
        elif rule not in output:
            print(f"FAIL {name}: rejected, but not for {rule}\n{output}")
            failures += 1
        else:
            print(f"ok   {name} -> {rule}")

    print(f"\n{len(CASES) + 1} checks, {failures} failing")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
