import { describe, expect, it } from "vitest";
import {
  FULL_INTENSITY_PCT,
  gridFor,
  heatColor,
  tileAt,
  tileRect,
  tileText,
} from "./heatmapLayout";

describe("heatmap layout", () => {
  it("makes a grid that holds every tile with roughly wide tiles", () => {
    for (const [n, w, h] of [
      [50, 700, 220],
      [13, 400, 300],
      [1, 100, 100],
      [100, 1400, 400],
    ] as const) {
      const g = gridFor(n, w, h);
      expect(g.cols * g.rows).toBeGreaterThanOrEqual(n);
      expect(g.cols).toBeGreaterThanOrEqual(1);
      expect(g.cols).toBeLessThanOrEqual(n);
      const aspect = w / g.cols / (h / g.rows);
      expect(aspect).toBeGreaterThan(0.7);
      expect(aspect).toBeLessThan(3.5);
    }
  });

  it("copes with nothing to draw", () => {
    expect(gridFor(0, 100, 100)).toEqual({ cols: 1, rows: 1 });
    expect(gridFor(5, 0, 100)).toEqual({ cols: 1, rows: 1 });
  });

  it("places tiles row by row inside the area and finds them again by point", () => {
    const grid = { cols: 4, rows: 3 };
    const [w, h] = [400, 300];
    expect(tileRect(0, grid, w, h)).toEqual({ x: 0.5, y: 0.5, w: 99, h: 99 });
    expect(tileRect(5, grid, w, h)).toMatchObject({ x: 100.5, y: 100.5 });
    for (let i = 0; i < 10; i++) {
      const r = tileRect(i, grid, w, h);
      expect(tileAt(r.x + r.w / 2, r.y + r.h / 2, 10, grid, w, h)).toBe(i);
    }
    // The last row is partly empty: points beyond the tiles hit nothing.
    expect(tileAt(399, 299, 10, grid, w, h)).toBe(-1);
    expect(tileAt(-1, 5, 10, grid, w, h)).toBe(-1);
    expect(tileAt(5, 300, 10, grid, w, h)).toBe(-1);
  });
});

describe("heat colours and labels", () => {
  const channels = (c: string) => c.match(/\d+/g)?.map(Number) ?? [];

  it("is neutral at zero, greener for gains and redder for losses, capped", () => {
    const zero = channels(heatColor(0));
    const up = channels(heatColor(1.5));
    const down = channels(heatColor(-1.5));
    const topUp = channels(heatColor(FULL_INTENSITY_PCT));
    expect((up[1] ?? 0) > (zero[1] ?? 0)).toBe(true);
    expect((down[0] ?? 0) > (zero[0] ?? 0)).toBe(true);
    expect(heatColor(50)).toBe(heatColor(FULL_INTENSITY_PCT));
    expect(topUp[1]).toBeGreaterThan(up[1] ?? 0);
    expect(heatColor(null)).toBe("#141414");
    expect(heatColor(Number.NaN)).toBe("#141414");
  });

  it("always writes the direction on the tile, not just a colour", () => {
    expect(tileText(1.256)).toBe("▲ 1.26%");
    expect(tileText(-0.4)).toBe("▼ 0.40%");
    expect(tileText(0)).toBe("– 0.00%");
    expect(tileText(null)).toBe("—");
  });
});
