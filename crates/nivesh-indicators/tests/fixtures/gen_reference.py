#!/usr/bin/env python3
"""Regenerates reference.json: independent, plain-Python implementations of
the indicators, run on a seeded synthetic series. They are deliberately
written as naive per-point formulas (no recurrences shared with the Rust
code), so a disagreement means one of the two is wrong.

    python3 -I crates/nivesh-indicators/tests/fixtures/gen_reference.py
"""
import json
import math
import os
import random

random.seed(20261118)

# 4 IST sessions of 5-minute bars (09:15-15:30 = 75 bars each).
IST = 19800
bars, price = [], 2500.0
day0 = 1_794_000_000 - (1_794_000_000 % 86400) - IST  # an IST midnight (UTC secs)
for day in range(4):
    open_t = day0 + day * 86400 + 9 * 3600 + 15 * 60
    for i in range(75):
        o = price
        c = max(1.0, o + random.gauss(0, 6))
        h = max(o, c) + abs(random.gauss(0, 3))
        l = min(o, c) - abs(random.gauss(0, 3))
        v = float(random.randint(1000, 90000))
        bars.append((open_t + i * 300, o, h, l, c, v))
        price = c

t = [b[0] for b in bars]
o = [b[1] for b in bars]
h = [b[2] for b in bars]
l = [b[3] for b in bars]
c = [b[4] for b in bars]
v = [b[5] for b in bars]
N = len(c)
nan = None  # JSON null


def sma(x, n):
    return [nan if i + 1 < n else sum(x[i + 1 - n : i + 1]) / n for i in range(len(x))]


def wma(x, n):
    d = n * (n + 1) / 2
    return [
        nan if i + 1 < n else sum((k + 1) * x[i + 1 - n + k] for k in range(n)) / d
        for i in range(len(x))
    ]


def ema(x, n, start=0):
    """SMA-seeded EMA over x[start:]; indices before the seed are None."""
    out = [nan] * len(x)
    a = 2 / (n + 1)
    seed = start + n - 1
    if seed >= len(x):
        return out
    out[seed] = sum(x[start : start + n]) / n
    for i in range(seed + 1, len(x)):
        out[i] = out[i - 1] + a * (x[i] - out[i - 1])
    return out


ref = {
    "t": t, "o": o, "h": h, "l": l, "c": c, "v": v,
    "sma_5": sma(c, 5), "sma_20": sma(c, 20),
    "ema_5": ema(c, 5), "ema_20": ema(c, 20),
    "wma_5": wma(c, 5), "wma_20": wma(c, 20),
}
with open(os.path.join(os.path.dirname(__file__), "reference.json"), "w") as f:
    json.dump(ref, f, separators=(",", ":"))
