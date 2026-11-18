// Layout and colour for the canvas heatmap. Pure, so it is unit tested; the
// component only draws what these return.

export interface Tile {
  id: string;
  label: string;
  /** Percent change, or null when unknown. */
  pct: number | null;
}

export interface Grid {
  cols: number;
  rows: number;
}

/** Tiles are drawn about this much wider than tall, which suits symbol labels. */
const TILE_ASPECT = 1.6;

/** The grid for `n` tiles in a `w` by `h` area, with roughly TILE_ASPECT tiles. */
export function gridFor(n: number, w: number, h: number): Grid {
  if (n <= 0 || w <= 0 || h <= 0) return { cols: 1, rows: 1 };
  const cols = Math.max(1, Math.min(n, Math.round(Math.sqrt((n * w) / (h * TILE_ASPECT)))));
  return { cols, rows: Math.ceil(n / cols) };
}

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Tile `i` in row-major order, with a 1 px gap between tiles. */
export function tileRect(i: number, grid: Grid, w: number, h: number, gap = 1): Rect {
  const cw = w / grid.cols;
  const ch = h / grid.rows;
  const col = i % grid.cols;
  const row = Math.floor(i / grid.cols);
  return { x: col * cw + gap / 2, y: row * ch + gap / 2, w: cw - gap, h: ch - gap };
}

/** The tile under a point, or -1. */
export function tileAt(x: number, y: number, n: number, grid: Grid, w: number, h: number): number {
  if (x < 0 || y < 0 || x >= w || y >= h) return -1;
  const col = Math.floor(x / (w / grid.cols));
  const row = Math.floor(y / (h / grid.rows));
  const i = row * grid.cols + col;
  return i < n ? i : -1;
}

/** A change at or beyond this is drawn at full intensity. */
export const FULL_INTENSITY_PCT = 3;

const NEUTRAL: [number, number, number] = [31, 31, 31];
const UP: [number, number, number] = [18, 128, 74];
const DOWN: [number, number, number] = [170, 40, 40];

/**
 * Tile colour: dark grey at 0%, shading to green or red as the change grows.
 * Direction is also written on the tile (▲/▼), so colour is never the only cue.
 */
export function heatColor(pct: number | null): string {
  if (pct === null || !Number.isFinite(pct)) return "#141414";
  const t = Math.min(1, Math.abs(pct) / FULL_INTENSITY_PCT);
  const target = pct >= 0 ? UP : DOWN;
  const mix = (i: 0 | 1 | 2) => Math.round(NEUTRAL[i] + (target[i] - NEUTRAL[i]) * t);
  return `rgb(${mix(0)}, ${mix(1)}, ${mix(2)})`;
}

/** `▲ 1.25%` / `▼ 0.40%`. */
export function tileText(pct: number | null): string {
  if (pct === null || !Number.isFinite(pct)) return "—";
  const arrow = pct > 0 ? "▲" : pct < 0 ? "▼" : "–";
  return `${arrow} ${Math.abs(pct).toFixed(2)}%`;
}
