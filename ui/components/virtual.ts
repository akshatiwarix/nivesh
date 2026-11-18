// Windowing arithmetic for long fixed-row-height tables. Pure, so it is unit
// tested; DataTable only wires it to the scroll container.

/** Tables with more rows than this render only the rows near the viewport. */
export const VIRTUALIZE_ABOVE = 100;

/** Rows drawn beyond each edge of the viewport, so fast scrolling never shows a gap. */
export const OVERSCAN = 8;

export interface Window {
  /** First rendered row (inclusive). */
  start: number;
  /** One past the last rendered row. */
  end: number;
}

export interface WindowInput {
  count: number;
  rowHeight: number;
  /** Scroll offset of the scroll container. */
  scrollTop: number;
  /** Visible height of the scroll container. */
  viewport: number;
  /** Distance from the top of the scrolled content to the first row (header, other content above). */
  offsetTop: number;
}

/** The rows to render for a scroll position, with overscan. */
export function visibleWindow(i: WindowInput): Window {
  if (i.count <= 0 || i.rowHeight <= 0) return { start: 0, end: 0 };
  const top = Math.max(0, i.scrollTop - i.offsetTop);
  const first = Math.floor(top / i.rowHeight);
  const last = Math.ceil((top + Math.max(0, i.viewport)) / i.rowHeight);
  return {
    start: Math.max(0, Math.min(i.count, first - OVERSCAN)),
    end: Math.max(0, Math.min(i.count, last + OVERSCAN)),
  };
}

/** Heights of the spacer rows standing in for the rows not rendered. */
export function spacers(w: Window, count: number, rowHeight: number) {
  return { before: w.start * rowHeight, after: Math.max(0, count - w.end) * rowHeight };
}

/**
 * The scroll offset that brings row `index` fully into view, or `scrollTop`
 * unchanged if it already is. `headerHeight` is the sticky header covering the
 * top of the viewport.
 */
export function scrollToReveal(
  index: number,
  i: Pick<WindowInput, "rowHeight" | "scrollTop" | "viewport" | "offsetTop"> & {
    headerHeight: number;
  },
): number {
  const rowTop = i.offsetTop + index * i.rowHeight;
  const rowBottom = rowTop + i.rowHeight;
  if (rowTop - i.headerHeight < i.scrollTop) return Math.max(0, rowTop - i.headerHeight);
  if (rowBottom > i.scrollTop + i.viewport) return rowBottom - i.viewport;
  return i.scrollTop;
}
