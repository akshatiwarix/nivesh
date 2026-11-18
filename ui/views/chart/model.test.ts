import { describe, expect, it } from "vitest";
import type { HistoryDto } from "../../ipc";
import {
  axisLabel,
  barAt,
  candlePoints,
  crosshairLabel,
  indexOfTime,
  initialRange,
  isIntraday,
  panRange,
  parseChartArgs,
  parseInterval,
  parseRange,
  stepInterval,
  volumePoints,
  zoomRange,
} from "./model";

const history: HistoryDto = {
  exchange: "NSE",
  symbol: "TCS",
  interval: "5m",
  range: "1mo",
  t: [1_000_000, 1_300_000, 1_600_000],
  o: [10, 11, 12],
  h: [12, 12, 13],
  l: [9, 10, 11],
  c: [11, 10.5, 12.5],
  v: [100, 200, 300],
  provenance: {
    provider: "yahoo",
    freshness: "DELAYED",
    asOfMs: 1_600_000,
    fetchedAtMs: 1_600_001,
  },
};

describe("chart arguments", () => {
  it("parses intervals and their shorthands, but not a bare M", () => {
    expect(parseInterval("5m")).toBe("5m");
    expect(parseInterval("1H")).toBe("1h");
    expect(parseInterval("4h")).toBe("4h");
    expect(parseInterval("D")).toBe("1d");
    expect(parseInterval("W")).toBe("1w");
    expect(parseInterval("MO")).toBe("1mo");
    expect(parseInterval("1MO")).toBe("1mo");
    expect(parseInterval("M")).toBeUndefined();
    expect(parseInterval("7m")).toBeUndefined();
  });

  it("parses ranges", () => {
    expect(parseRange("1MO")).toBe("1mo");
    expect(parseRange("MAX")).toBe("max");
    expect(parseRange("2y")).toBeUndefined();
  });

  it("takes the interval first and the range second", () => {
    expect(parseChartArgs([])).toEqual({ interval: "1d" });
    expect(parseChartArgs(["5M"])).toEqual({ interval: "5m" });
    expect(parseChartArgs(["1D", "5Y"])).toEqual({ interval: "1d", range: "5y" });
    expect(parseChartArgs(["x"])).toMatch(/Unknown interval "x"/);
    expect(parseChartArgs(["5m", "x"])).toMatch(/Unknown range "x"/);
    expect(parseChartArgs(["5m", "1mo", "extra"])).toMatch(/Usage/);
  });

  it("steps through intervals and stops at the ends", () => {
    expect(stepInterval("5m", 1)).toBe("15m");
    expect(stepInterval("5m", -1)).toBe("1m");
    expect(stepInterval("1m", -1)).toBe("1m");
    expect(stepInterval("1mo", 1)).toBe("1mo");
  });
});

describe("series shaping", () => {
  it("turns columns into candles with UTC seconds", () => {
    const c = candlePoints(history);
    expect(c).toHaveLength(3);
    expect(c[1]).toEqual({ time: 1300, open: 11, high: 12, low: 10, close: 10.5 });
  });

  it("tints volume by the bar's direction", () => {
    const colors = { up: "U", down: "D", upDim: "u", downDim: "d" };
    expect(volumePoints(history, colors).map((v) => v.color)).toEqual(["u", "d", "u"]);
  });

  it("finds bars by time", () => {
    expect(indexOfTime(history.t, 1_300_000)).toBe(1);
    expect(indexOfTime(history.t, 1_300_001)).toBe(-1);
    expect(indexOfTime([], 5)).toBe(-1);
    expect(barAt(history, 2)).toEqual({ t: 1_600_000, o: 12, h: 13, l: 11, c: 12.5, v: 300 });
    expect(barAt(history, 3)).toBeUndefined();
  });
});

describe("IST time labels", () => {
  // 2026-11-16 09:15 IST = 03:45 UTC.
  const open = Date.UTC(2026, 10, 16, 3, 45) / 1000;

  it("shows IST wall time for intraday ticks and dates otherwise", () => {
    expect(axisLabel(open, true, false)).toBe("09:15");
    expect(axisLabel(open, true, true)).toBe("16 Nov");
    expect(axisLabel(open, false, false)).toBe("16 Nov");
    expect(crosshairLabel(open, true)).toBe("16 Nov 2026 09:15");
    expect(crosshairLabel(open, false)).toBe("16 Nov 2026");
  });

  it("knows which intervals are intraday", () => {
    expect(isIntraday("5m")).toBe(true);
    expect(isIntraday("4h")).toBe(true);
    expect(isIntraday("1d")).toBe(false);
    expect(isIntraday("1mo")).toBe(false);
  });
});

describe("zoom and pan", () => {
  it("zooms about the right edge and keeps a sane minimum", () => {
    expect(zoomRange({ from: 0, to: 100 }, 0.5, 1000)).toEqual({ from: 50, to: 100 });
    const tight = zoomRange({ from: 90, to: 100 }, 0.1, 1000);
    expect(tight.to - tight.from).toBe(5);
    const wide = zoomRange({ from: 0, to: 1000 }, 10, 100);
    expect(wide.to - wide.from).toBe(200);
  });

  it("pans by a fraction of the width", () => {
    expect(panRange({ from: 100, to: 200 }, -0.2)).toEqual({ from: 80, to: 180 });
    expect(panRange({ from: 100, to: 200 }, 0.5)).toEqual({ from: 150, to: 250 });
  });

  it("opens on the newest bars with room on the right", () => {
    expect(initialRange(1000)).toEqual({ from: 850, to: 1004 });
    expect(initialRange(20)).toEqual({ from: 0, to: 24 });
  });
});
