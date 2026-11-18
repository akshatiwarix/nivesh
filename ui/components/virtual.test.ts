import { describe, expect, it } from "vitest";
import { OVERSCAN, scrollToReveal, spacers, VIRTUALIZE_ABOVE, visibleWindow } from "./virtual";

const base = { count: 1000, rowHeight: 18, scrollTop: 0, viewport: 360, offsetTop: 40 };

describe("visibleWindow", () => {
  it("shows the first rows plus overscan at the top", () => {
    // 360 px / 18 px = 20 rows, plus overscan below; nothing above.
    expect(visibleWindow(base)).toEqual({ start: 0, end: 20 + OVERSCAN });
  });

  it("follows the scroll position", () => {
    // 40 px of content above, scrolled 40 + 1800 px: row 100 is at the top.
    const w = visibleWindow({ ...base, scrollTop: 1840 });
    expect(w).toEqual({ start: 100 - OVERSCAN, end: 120 + OVERSCAN });
  });

  it("clamps to the table at the end and handles short or empty tables", () => {
    expect(visibleWindow({ ...base, scrollTop: 1e6 }).end).toBe(1000);
    expect(visibleWindow({ ...base, count: 5 })).toEqual({ start: 0, end: 5 });
    expect(visibleWindow({ ...base, count: 0 })).toEqual({ start: 0, end: 0 });
    expect(visibleWindow({ ...base, rowHeight: 0 })).toEqual({ start: 0, end: 0 });
  });

  it("renders a bounded number of rows however long the table is", () => {
    for (const count of [101, 5_000, 1_000_000]) {
      const w = visibleWindow({ ...base, count, scrollTop: 18 * 500 });
      expect(w.end - w.start).toBeLessThanOrEqual(20 + 2 * OVERSCAN + 1);
    }
  });
});

describe("spacers and reveal", () => {
  it("spacers stand in for every row not rendered", () => {
    const w = { start: 90, end: 130 };
    const s = spacers(w, 1000, 18);
    expect(s.before + (w.end - w.start) * 18 + s.after).toBe(1000 * 18);
  });

  it("scrolls the minimum to bring a row into view, and not at all if it is visible", () => {
    const i = { rowHeight: 18, scrollTop: 1000, viewport: 360, offsetTop: 40, headerHeight: 18 };
    // Visible: row 60 spans 1120-1138.
    expect(scrollToReveal(60, i)).toBe(1000);
    // Above the viewport: bring it just under the sticky header.
    expect(scrollToReveal(10, i)).toBe(40 + 180 - 18);
    // Below: align its bottom edge with the viewport's.
    expect(scrollToReveal(100, i)).toBe(40 + 100 * 18 + 18 - 360);
    expect(scrollToReveal(0, { ...i, scrollTop: 0 })).toBe(0);
  });

  it("virtualises only long tables", () => {
    expect(VIRTUALIZE_ABOVE).toBe(100);
  });
});
