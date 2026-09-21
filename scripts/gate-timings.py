#!/usr/bin/env python3
"""Run a gate table's rows in order, timing each.

    scripts/gate-timings.py [REPO]

The gate keeps a log only for rows that FAIL, so a passing run leaves no record
of where its minutes went, and "the gate feels slow" gets answered by guessing.
This runs the same argv, in the same order, in the same cwd, with the same
environment, and reports a duration per row.

⚠ **The environment is part of the row, and leaving it out does not merely
mis-measure — it THRASHES.** An earlier copy of this ignored `row["env"]`, so
the rustdoc row ran without `RUSTDOCFLAGS`. That is a different fingerprint from
the one the real gate builds, so each run invalidated the other's and the row
rebuilt from scratch every time: 94s measured against 0.4s for the same command
run properly. Three of memview#1673's per-row figures for that row came from the
broken version, and the conclusions drawn from them were about the instrument.

⚠ **Run it when no gate is running.** The rows share build caches and a CPU, and
two at once measures contention.

⚠ **Wall time, not CPU.** Rows that shell into `nix develop` pay for the shell,
and a cold nix evaluation is not the row's own cost.
"""

import json
import os
import subprocess
import sys
import time

root = sys.argv[1] if len(sys.argv) > 1 else "."
table = json.load(open(f"{root}/gate.json"))
rows = table if isinstance(table, list) else table.get("rows", table.get("checks", []))

timed = []
for n, row in enumerate(rows, 1):
    # Extended, never replaced: a row inherits PATH, HOME and the nix profile,
    # and its own `env` is the delta the table states.
    env = {**os.environ, **row.get("env", {})}
    began = time.monotonic()
    done = subprocess.run(
        row["argv"],
        cwd=f"{root}/{row.get('cwd', '.')}",
        env=env,
        capture_output=True,
        text=True,
    )
    took = time.monotonic() - began
    timed.append((took, n, row["name"], done.returncode))
    print(f"{took:7.1f}s  {n:>3}. {row['name']} (exit {done.returncode})", flush=True)

total = sum(t for t, *_ in timed)
print(f"\nTOTAL {total / 60:.1f} minutes over {len(timed)} rows\n")
print("The heaviest:")
for took, n, name, code in sorted(timed, reverse=True)[:8]:
    print(f"  {took:7.1f}s  ({took / total * 100:4.1f}%)  {n:>3}. {name}")
