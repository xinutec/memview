"""Write python-floats.jsonl: what CPython prints for floats, for floats.rs.

    python3 reader/tests/suite/python-floats.py > reader/tests/suite/python-floats.jsonl

Random doubles across every exponent, decimals of a few places, and exact
binary ties, which is where rounding rules differ.
"""

import json
import random
import struct

random.seed(20261105)
values = [
    0.0, -0.0, 1.0, -1.0, 0.1, 0.2, 0.3, 1 / 3, 2 / 3, 0.5, 1.5, 2.5, 0.25,
    0.35, 0.125, 0.0185, 1e16, 1e15, 9.999999999999999e15, 1e-4, 1e-5, 1e22,
    1e300, 5e-324, 1.7976931348623157e308, 2.0**53, 2.0**53 + 2, 0.1 + 0.2,
    100.0, 1234.5, 2.675, 1.005, 0.045, 1e-7, 12345678901234567.0,
]
for _ in range(1500):
    values.append(struct.unpack("<d", struct.pack("<Q", random.getrandbits(64)))[0])
for _ in range(800):
    values.append(round(random.uniform(-1000, 1000), random.randint(0, 6)))
for _ in range(400):
    values.append(random.randint(-(10**6), 10**6) / 2 ** random.randint(1, 12))
for v in values:
    if v != v or abs(v) == float("inf"):
        continue
    row = {"repr": repr(v), "round2": repr(round(v, 2))}
    if abs(v) < 1e20:
        row["fixed3"] = f"{v:.3f}"
    print(json.dumps(row))
