#!/usr/bin/env nix-shell
#!nix-shell -i python3 -p python3
"""Repoint one transcript line's `parentUuid`, in place, without moving a byte.

Why this exists rather than a rewrite-and-rename: a session is never finished
(see the sessions-are-permanent note) and the CLI appends to its transcript by
open/append/close. Renaming a rebuilt file over the original races those appends
and silently drops the ones that land mid-swap. A uuid is 36 characters and so
is its replacement, so the edit fits exactly where the old value sat: seek,
write 36 bytes, done. File length never changes and no append can be lost.

The script refuses to guess. It is told the line, the value it expects to find
there and the value to write, and it verifies the expectation at the byte level
before touching anything. Default is a dry run; --apply is required to write.

    ./reparent-transcript.py --file T.jsonl --line 149525 \
        --expect-old <uuid> --new <uuid> [--apply]

--apply also appends a revert record (file, byte offset, old value) to the path
given by --revert-log, which is the whole backup: 36 bytes and an offset restore
the original exactly, where copying the file would cost gigabytes.
"""

import argparse
import json
import os
import sys

UUID_LEN = 36


def locate(path, lineno, old, new):
    """Byte offset of `old` within line `lineno`, plus facts worth asserting.

    Walks the file once. Returns the offset and, as a side observation, whether
    the replacement uuid actually exists as some line's `uuid` and how many
    lines already point at it -- the two things that decide whether the new
    parent is real and whether this creates a fork.
    """
    offset = 0
    found = None
    new_exists = False
    existing_children = 0
    target = old.encode()

    with open(path, "rb") as handle:
        for i, raw in enumerate(handle, 1):
            if i == lineno:
                count = raw.count(target)
                if count != 1:
                    sys.exit(
                        f"line {lineno} contains the old uuid {count} times, "
                        "expected exactly 1 -- refusing to guess which"
                    )
                found = offset + raw.index(target)
            stripped = raw.strip()
            if stripped:
                try:
                    node = json.loads(stripped)
                except json.JSONDecodeError:
                    node = None
                if isinstance(node, dict):
                    if node.get("uuid") == new:
                        new_exists = True
                    if node.get("parentUuid") == new:
                        existing_children += 1
            offset += len(raw)

    if found is None:
        sys.exit(f"file has fewer than {lineno} lines")
    return found, new_exists, existing_children


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--file", required=True)
    ap.add_argument("--line", type=int, required=True)
    ap.add_argument("--expect-old", required=True)
    ap.add_argument("--new", required=True)
    ap.add_argument("--apply", action="store_true")
    ap.add_argument("--revert-log", default=None)
    args = ap.parse_args()

    if len(args.expect_old) != UUID_LEN or len(args.new) != UUID_LEN:
        sys.exit(
            f"both uuids must be {UUID_LEN} characters -- the in-place edit "
            "depends on the replacement being the same width as the original"
        )

    offset, new_exists, children = locate(
        args.file, args.line, args.expect_old, args.new
    )

    # Read back what is actually on disk at the computed offset. The walk above
    # could be wrong; these 36 bytes cannot be.
    with open(args.file, "rb") as handle:
        handle.seek(offset)
        actual = handle.read(UUID_LEN).decode()
    if actual != args.expect_old:
        sys.exit(f"byte {offset} holds {actual!r}, expected {args.expect_old!r}")

    size = os.path.getsize(args.file)
    print(f"file    {args.file}")
    print(f"        {size / 1e9:.2f} GB, editing line {args.line} at byte {offset}")
    print(f"parent  {args.expect_old} -> {args.new}")
    print(f"        replacement exists as a uuid in this file: {new_exists}")
    print(f"        lines already parented to it: {children}")
    if not new_exists:
        sys.exit("refusing: the replacement parent is not a uuid in this file")

    if not args.apply:
        print("\ndry run -- nothing written. Pass --apply to make the edit.")
        return

    with open(args.file, "r+b") as handle:
        handle.seek(offset)
        handle.write(args.new.encode())

    with open(args.file, "rb") as handle:
        handle.seek(offset)
        confirmed = handle.read(UUID_LEN).decode()
    if confirmed != args.new:
        sys.exit(f"write did not stick: byte {offset} holds {confirmed!r}")
    if os.path.getsize(args.file) < size:
        sys.exit("file shrank -- this should be impossible, investigate")

    if args.revert_log:
        record = {
            "file": args.file,
            "line": args.line,
            "offset": offset,
            "restore": args.expect_old,
            "written": args.new,
        }
        with open(args.revert_log, "a") as handle:
            handle.write(json.dumps(record) + "\n")
        print(f"revert record appended to {args.revert_log}")

    print(f"applied: byte {offset} now holds {args.new}")


if __name__ == "__main__":
    main()
