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


def rsi(x, n):
    """Wilder RSI written from the definition: seed with the mean of the first
    n changes, then smooth. Returns None until index n."""
    out = [nan] * len(x)
    if len(x) <= n:
        return out
    ch = [x[i] - x[i - 1] for i in range(1, len(x))]
    ag = sum(max(d, 0) for d in ch[:n]) / n
    al = sum(max(-d, 0) for d in ch[:n]) / n

    def val(ag, al):
        if al == 0:
            return 50.0 if ag == 0 else 100.0
        return 100 - 100 / (1 + ag / al)

    out[n] = val(ag, al)
    for i in range(n, len(ch)):
        ag = (ag * (n - 1) + max(ch[i], 0)) / n
        al = (al * (n - 1) + max(-ch[i], 0)) / n
        out[i + 1] = val(ag, al)
    return out


def macd(x, f, s, g):
    ef, es = ema(x, f), ema(x, s)
    m = [nan if a is None or b is None else a - b for a, b in zip(ef, es)]
    first = next(i for i, v in enumerate(m) if v is not None)
    sig = ema(m, g, start=first)
    hist = [nan if a is None or b is None else a - b for a, b in zip(m, sig)]
    return m, sig, hist


def boll(x, n, k):
    mid, up, lo = [], [], []
    for i in range(len(x)):
        if i + 1 < n:
            mid.append(nan); up.append(nan); lo.append(nan)
            continue
        w = x[i + 1 - n : i + 1]
        mu = sum(w) / n
        sd = math.sqrt(sum((a - mu) ** 2 for a in w) / n)
        mid.append(mu); up.append(mu + k * sd); lo.append(mu - k * sd)
    return mid, up, lo


def atr(h, l, c, n):
    tr = [h[0] - l[0]] + [
        max(h[i] - l[i], abs(h[i] - c[i - 1]), abs(l[i] - c[i - 1]))
        for i in range(1, len(c))
    ]
    out = [nan] * len(c)
    out[n - 1] = sum(tr[:n]) / n
    for i in range(n, len(c)):
        out[i] = (out[i - 1] * (n - 1) + tr[i]) / n
    return out


def vwap(t, h, l, c, v):
    out, day, pv, vol = [], None, 0.0, 0.0
    for i in range(len(t)):
        d = (t[i] + IST) // 86400
        if d != day:
            day, pv, vol = d, 0.0, 0.0
        pv += (h[i] + l[i] + c[i]) / 3 * v[i]
        vol += v[i]
        out.append(pv / vol if vol > 0 else nan)
    return out


ref = {
    "t": [x * 1000 for x in t], "o": o, "h": h, "l": l, "c": c, "v": v,
    "sma_5": sma(c, 5), "sma_20": sma(c, 20),
    "ema_5": ema(c, 5), "ema_20": ema(c, 20),
    "wma_5": wma(c, 5), "wma_20": wma(c, 20),
}
ref["rsi_14"] = rsi(c, 14)
ref["rsi_5"] = rsi(c, 5)
ref["macd_line"], ref["macd_signal"], ref["macd_hist"] = macd(c, 12, 26, 9)
ref["boll_mid"], ref["boll_up"], ref["boll_lo"] = boll(c, 20, 2.0)
ref["atr_14"] = atr(h, l, c, 14)
ref["vwap"] = vwap(t, h, l, c, v)
with open(os.path.join(os.path.dirname(__file__), "reference.json"), "w") as f:
    json.dump(ref, f, separators=(",", ":"))
