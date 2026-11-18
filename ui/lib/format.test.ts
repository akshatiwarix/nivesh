import { describe, expect, it } from "vitest";
import {
  arrow,
  changeTone,
  formatChange,
  formatCount,
  formatMove,
  formatMovePct,
  formatPct,
  formatPrice,
  formatRange,
  rangePosition,
} from "./format";
import { formatAge, formatAsOf } from "./time";

describe("number formatting", () => {
  it("groups digits the Indian way", () => {
    expect(formatPrice(123456.5)).toBe("1,23,456.50");
    expect(formatPrice(25243.4)).toBe("25,243.40");
    expect(formatPrice(12345678.9)).toBe("1,23,45,678.90");
    expect(formatCount(12345678)).toBe("1,23,45,678");
  });

  it("shows an em dash for missing or non-finite values", () => {
    for (const v of [undefined, null, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(formatPrice(v)).toBe("—");
      expect(formatChange(v)).toBe("—");
      expect(formatPct(v)).toBe("—");
      expect(formatMove(v)).toBe("—");
    }
  });

  it("signs changes and pairs colour with arrows", () => {
    expect(formatChange(12.5)).toBe("+12.50");
    expect(formatChange(-3)).toBe("-3.00");
    expect(formatChange(0)).toBe("0.00");
    expect(formatPct(0.619)).toBe("+0.62%");
    expect(formatPct(-1.04)).toBe("-1.04%");
    expect(formatMove(12.5)).toBe("▲ +12.50");
    expect(formatMovePct(-0.563)).toBe("▼ -0.56%");
    expect([arrow(1), arrow(-1), arrow(0)]).toEqual(["▲", "▼", "–"]);
    expect([changeTone(1), changeTone(-1), changeTone(0), changeTone(undefined)]).toEqual([
      "up",
      "down",
      undefined,
      undefined,
    ]);
  });

  it("formats ranges and positions within them", () => {
    expect(formatRange(100, 200)).toBe("100.00 – 200.00");
    expect(formatRange(undefined, 200)).toBe("—");
    expect(rangePosition(150, 100, 200)).toBe(0.5);
    expect(rangePosition(50, 100, 200)).toBe(0);
    expect(rangePosition(500, 100, 200)).toBe(1);
    expect(rangePosition(150, 200, 200)).toBeUndefined();
    expect(rangePosition(undefined, 100, 200)).toBeUndefined();
  });
});

describe("as-of formatting", () => {
  // 2026-11-18 10:00:00 UTC = 15:30:00 IST.
  const close = Date.UTC(2026, 10, 18, 10, 0, 0);

  it("shows just the time for today's data (IST)", () => {
    expect(formatAsOf(close, close + 60_000)).toBe("15:30:00 IST");
  });

  it("adds the date once the data is from another IST day", () => {
    expect(formatAsOf(close, close + 24 * 3600_000)).toBe("18 NOV 15:30 IST");
  });

  it("uses the IST day, not the UTC day, for the boundary", () => {
    // 18:45 UTC on the 17th is 00:15 IST on the 18th: same IST day as `close`.
    const earlyIst = Date.UTC(2026, 10, 17, 18, 45, 0);
    expect(formatAsOf(earlyIst, close)).toBe("00:15:00 IST");
  });

  it("formats ages coarsely", () => {
    expect([0, 12_000, 59_999, 60_000, 3_600_000, 90_000_000].map(formatAge)).toEqual([
      "0s",
      "12s",
      "59s",
      "1m",
      "1h",
      "1d",
    ]);
    expect(formatAge(-5)).toBe("0s");
  });
});
