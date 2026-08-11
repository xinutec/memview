#!/usr/bin/env nix-shell
#!nix-shell -i python3 -p python3
"""Remove the transcripts left behind by the console's own gist calls.

Each sentence on a session card is written by a one-shot `claude -p`, and a
one-shot call is a conversation: it leaves a transcript under
~/.claude/projects/ like any other. Those calls run from the temp directory, so
theirs are filed in a project folder named for it -- which `past::conversations`
hides from the console's list. Nothing showed them, so nothing noticed 2,299 of
them, 57 MB, accumulating at a sweep's worth every quarter of an hour.

`gist::discard` now takes each one away as soon as its answer is in hand, so
nothing new accumulates. This is for what was already there, and for a machine
that runs an older build.

WHAT IT WILL AND WILL NOT DELETE. Only files whose first user message begins
with the gist prompt's own opening sentence -- which the console wrote and no
person would. A conversation that merely ran from a temporary directory is
somebody's, is resumable by id, and is left alone. Default is a dry run; --remove
is required to delete.

    ./gist-strays.py [--root ~/.claude/projects] [--remove]
"""

import argparse
import json
import os
import sys

# The first words of `gist::prompt`. Anything opening with these was asked for
# by this console; anything else was asked for by a person.
OPENING = "Below is part of a conversation between a person and a coding agent"


def first_words(path):
    """What the first user message of a transcript says, or None.

    Reads until it finds one rather than parsing the file: a gist call's own
    transcript is one exchange, and the prompt is at the top of it. Anything
    unreadable -- half-written, not JSON, not a transcript at all -- is None,
    which means "leave it alone".
    """
    try:
        with open(path, encoding="utf-8", errors="replace") as handle:
            for line in handle:
                try:
                    entry = json.loads(line)
                except ValueError:
                    continue
                if entry.get("type") != "user":
                    continue
                content = entry.get("message", {}).get("content")
                if isinstance(content, str):
                    return content
                if isinstance(content, list):
                    for part in content:
                        if part.get("type") == "text":
                            return part.get("text", "")
                return None
    except OSError:
        return None
    return None


def strays(root):
    """Every transcript under `root` that this console wrote to ask a question."""
    found = []
    for project in sorted(os.listdir(root)):
        folder = os.path.join(root, project)
        if not os.path.isdir(folder):
            continue
        for name in sorted(os.listdir(folder)):
            if not name.endswith(".jsonl"):
                continue
            path = os.path.join(folder, name)
            opening = first_words(path)
            if opening and opening.startswith(OPENING):
                found.append(path)
    return found


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        default=os.path.expanduser("~/.claude/projects"),
        help="where the transcripts are (default: ~/.claude/projects)",
    )
    parser.add_argument(
        "--remove",
        action="store_true",
        help="actually delete them; without this it only says what it would",
    )
    args = parser.parse_args()

    found = strays(args.root)
    total = sum(os.path.getsize(path) for path in found)
    if not found:
        print("no gist transcripts left behind")
        return 0

    print(f"{len(found)} gist transcript(s), {total / 1e6:.1f} MB")
    if not args.remove:
        for path in found[:5]:
            print("  would remove", path)
        if len(found) > 5:
            print(f"  ... and {len(found) - 5} more")
        print("dry run -- pass --remove to delete them")
        return 0

    gone = 0
    for path in found:
        try:
            os.remove(path)
            gone += 1
        except OSError as why:
            print(f"  {path}: {why}", file=sys.stderr)
    print(f"removed {gone} of {len(found)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
