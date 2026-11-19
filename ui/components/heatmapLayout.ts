// Layout and colour for the canvas heatmap. Pure, so it is unit tested; the
// component only draws what these return.

export interface Tile {
  id: string;
  label: string;
  /** Percent change, or null when unknown. */
  pct: number | null;
  /**
   * What the tile's area tracks (e.g. last session's traded value), or null
   * for an equal share. Absent weights fall back to equal shares too.
   */
  weight?: number | null;
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

/** A weight counts only when it is a positive, finite number. */
function usableWeight(tile: Tile): number {
  const v = tile.weight;
  return v !== undefined && v !== null && Number.isFinite(v) && v > 0 ? v : 0;
}

/**
 * Rects for all tiles, in input order. Tiles without weights share the area
 * equally (the plain grid); with weights, areas follow a squarified greedy
 * fill along the shorter side so a tile's area tracks its weight — e.g.
 * traded value — while rows stay reasonably square. Very small weights are
 * clamped up to a quarter of the equal share so a label still fits.
 */
export function layoutFor(tiles: readonly Tile[], w: number, h: number, gap = 1): Rect[] {
  const n = tiles.length;
  if (n === 0 || w <= 0 || h <= 0) return [];
  const weights = tiles.map(usableWeight);
  const total = weights.reduce((a, b) => a + b, 0);
  if (total <= 0) {
    const grid = gridFor(n, w, h);
    return tiles.map((_, i) => tileRect(i, grid, w, h, gap));
  }

  // Shares of the whole area. Tiny ones are clamped up and huge ones down,
  // so no tile is smaller than a quarter of the equal share (a label still
  // fits) and the floor survives the renormalisation.
  const minShare = 1 / n / 4;
  const maxShare = 1 - minShare * (n - 1);
  const clamped = weights.map((v) => {
    const share = v / total;
    return Math.min(Math.max(share, minShare), maxShare);
  });
  const clampedTotal = clamped.reduce((a, b) => a + b, 0);
  const areas = clamped.map((s) => (s / clampedTotal) * w * h);

  const rects: Rect[] = new Array(n);
  const worstOf = (row: number[], sum: number, length: number): number => {
    const t = sum / length;
    let worst = 0;
    for (const j of row) {
      const l = ((areas[j] ?? 0) / sum) * length;
      worst = Math.max(worst, t / l, l / t);
    }
    return worst;
  };
  let ox = 0;
  let oy = 0;
  let ow = w;
  let oh = h;
  let idx = 0;
  while (idx < n) {
    // A wide remaining rectangle takes a vertical strip on the left.
    const vertical = ow >= oh;
    const length = vertical ? oh : ow;
    let row = [idx];
    let sum = areas[idx] ?? 0;
    let worst = worstOf(row, sum, length);
    while (idx + row.length < n) {
      const next = idx + row.length;
      const candSum = sum + (areas[next] ?? 0);
      const candWorst = worstOf([...row, next], candSum, length);
      if (candWorst <= worst) {
        row = [...row, next];
        sum = candSum;
        worst = candWorst;
      } else {
        break;
      }
    }
    // Thickness of the strip along the other side, clamped to what is left.
    const thickness = Math.min(sum / length, vertical ? ow : oh);
    if (vertical) {
      let y = oy;
      for (const j of row) {
        const ch = Math.min(((areas[j] ?? 0) / sum) * oh, oy + oh - y);
        rects[j] = { x: ox, y, w: thickness, h: ch };
        y += ch;
      }
      ox += thickness;
      ow -= thickness;
    } else {
      let x = ox;
      for (const j of row) {
        const cw = Math.min(((areas[j] ?? 0) / sum) * ow, ox + ow - x);
        rects[j] = { x, y: oy, w: cw, h: thickness };
        x += cw;
      }
      oy += thickness;
      oh -= thickness;
    }
    idx += row.length;
  }
  // The 1 px gap, inset on every side like the plain grid does.
  return rects.map((r) => ({
    x: r.x + gap / 2,
    y: r.y + gap / 2,
    w: Math.max(0, r.w - gap),
    h: Math.max(0, r.h - gap),
  }));
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
