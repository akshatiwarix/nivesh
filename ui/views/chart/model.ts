// Pure helpers for the chart view: argument parsing, series shaping, time
// formatting in IST and the zoom/pan arithmetic. No DOM, so it is unit tested.

import type { HistoryDto } from "../../ipc";

/** Chart intervals, finest first. Codes are the ones the backend takes. */
export const INTERVALS = ["1m", "5m", "15m", "30m", "1h", "4h", "1d", "1w", "1mo"] as const;
export type ChartInterval = (typeof INTERVALS)[number];

export const RANGES = ["1d", "5d", "1mo", "3mo", "6mo", "1y", "5y", "max"] as const;
export type ChartRange = (typeof RANGES)[number];

/** What the toolbar shows for each interval. */
export const INTERVAL_LABEL: Record<ChartInterval, string> = {
  "1m": "1M",
  "5m": "5M",
  "15m": "15M",
  "30m": "30M",
  "1h": "1H",
  "4h": "4H",
  "1d": "1D",
  "1w": "1W",
  "1mo": "1MO",
};

export const DEFAULT_INTERVAL: ChartInterval = "1d";

const INTERVAL_ALIASES: Record<string, ChartInterval> = {
  D: "1d",
  W: "1w",
  H: "1h",
  MO: "1mo",
  MN: "1mo",
};

/** `5M`, `1H`, `D`, `1MO` -> interval code. `M` alone is ambiguous, so it is not accepted. */
export function parseInterval(text: string): ChartInterval | undefined {
  const t = text.trim().toUpperCase();
  const alias = INTERVAL_ALIASES[t];
  if (alias) return alias;
  const code = t.toLowerCase();
  return (INTERVALS as readonly string[]).includes(code) ? (code as ChartInterval) : undefined;
}

export function parseRange(text: string): ChartRange | undefined {
  const code = text.trim().toLowerCase();
  return (RANGES as readonly string[]).includes(code) ? (code as ChartRange) : undefined;
}

export interface ChartArgs {
  interval: ChartInterval;
  /** `undefined` means the default for the interval (decided by the backend). */
  range?: ChartRange;
}

/**
 * `CH`, `CH 5M`, `CH 5M 1MO`: the first argument is the interval and the
 * second the range. Returns an error message for the command bar on bad input.
 */
export function parseChartArgs(args: readonly string[]): ChartArgs | string {
  const [first, second, ...extra] = args;
  const interval = first === undefined ? DEFAULT_INTERVAL : parseInterval(first);
  if (!interval) {
    return `Unknown interval "${first}". Use ${INTERVALS.map((i) => INTERVAL_LABEL[i]).join(", ")}.`;
  }
  let range: ChartRange | undefined;
  if (second !== undefined) {
    range = parseRange(second);
    if (!range)
      return `Unknown range "${second}". Use ${RANGES.map((r) => r.toUpperCase()).join(", ")}.`;
  }
  if (extra.length > 0) return "Usage: <SYMBOL> CH [INTERVAL] [RANGE], e.g. RELIANCE CH 5M 1MO";
  return range ? { interval, range } : { interval };
}

/** The next (+1) or previous (-1) interval, stopping at the ends. */
export function stepInterval(current: ChartInterval, delta: 1 | -1): ChartInterval {
  const i = INTERVALS.indexOf(current);
  const next = Math.min(INTERVALS.length - 1, Math.max(0, i + delta));
  return INTERVALS[next] ?? current;
}

// ---- series shaping --------------------------------------------------------

export interface CandlePoint {
  /** UTC seconds, as the chart library wants. */
  time: number;
  open: number;
  high: number;
  low: number;
  close: number;
}

export interface VolumePoint {
  time: number;
  value: number;
  color: string;
}

export interface Colors {
  up: string;
  down: string;
  upDim: string;
  downDim: string;
}

export function candlePoints(h: HistoryDto): CandlePoint[] {
  const out: CandlePoint[] = new Array(h.t.length);
  for (let i = 0; i < h.t.length; i++) {
    out[i] = {
      time: (h.t[i] as number) / 1000,
      open: h.o[i] as number,
      high: h.h[i] as number,
      low: h.l[i] as number,
      close: h.c[i] as number,
    };
  }
  return out;
}

/** Volume bars, tinted by whether the bar closed up or down. */
export function volumePoints(h: HistoryDto, colors: Colors): VolumePoint[] {
  const out: VolumePoint[] = new Array(h.t.length);
  for (let i = 0; i < h.t.length; i++) {
    const up = (h.c[i] as number) >= (h.o[i] as number);
    out[i] = {
      time: (h.t[i] as number) / 1000,
      value: h.v[i] as number,
      color: up ? colors.upDim : colors.downDim,
    };
  }
  return out;
}

/** Index of the bar starting at `tMs`, or -1. Binary search over the sorted `t` column. */
export function indexOfTime(t: readonly number[], tMs: number): number {
  let lo = 0;
  let hi = t.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const v = t[mid] as number;
    if (v === tMs) return mid;
    if (v < tMs) lo = mid + 1;
    else hi = mid - 1;
  }
  return -1;
}

export interface Bar {
  t: number;
  o: number;
  h: number;
  l: number;
  c: number;
  v: number;
}

export function barAt(h: HistoryDto, i: number): Bar | undefined {
  if (i < 0 || i >= h.t.length) return undefined;
  return {
    t: h.t[i] as number,
    o: h.o[i] as number,
    h: h.h[i] as number,
    l: h.l[i] as number,
    c: h.c[i] as number,
    v: h.v[i] as number,
  };
}

// ---- IST time ---------------------------------------------------------------

const IST_SECS = 19_800;
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const pad = (n: number) => String(n).padStart(2, "0");

function ist(timeSecs: number) {
  const d = new Date((timeSecs + IST_SECS) * 1000);
  return {
    y: d.getUTCFullYear(),
    mo: d.getUTCMonth(),
    d: d.getUTCDate(),
    h: d.getUTCHours(),
    mi: d.getUTCMinutes(),
  };
}

/** Axis label: the time within a day for intraday ticks, the date otherwise. */
export function axisLabel(timeSecs: number, intraday: boolean, newDay: boolean): string {
  const f = ist(timeSecs);
  if (intraday && !newDay) return `${pad(f.h)}:${pad(f.mi)}`;
  return `${f.d} ${MONTHS[f.mo]}`;
}

/** Crosshair label: full date, plus the time for intraday bars. */
export function crosshairLabel(timeSecs: number, intraday: boolean): string {
  const f = ist(timeSecs);
  const date = `${pad(f.d)} ${MONTHS[f.mo]} ${f.y}`;
  return intraday ? `${date} ${pad(f.h)}:${pad(f.mi)}` : date;
}

export function isIntraday(interval: string): boolean {
  return !["1d", "1w", "1mo"].includes(interval);
}

// ---- zoom and pan -----------------------------------------------------------

export interface LogicalRange {
  from: number;
  to: number;
}

const MIN_BARS = 5;

/** Scales the visible span by `factor` (< 1 zooms in) about the right edge. */
export function zoomRange(r: LogicalRange, factor: number, totalBars: number): LogicalRange {
  const span = r.to - r.from;
  const next = Math.min(Math.max(MIN_BARS, span * factor), Math.max(MIN_BARS, totalBars * 2));
  return { from: r.to - next, to: r.to };
}

/** Shifts the visible range by `fraction` of its width (negative = back in time). */
export function panRange(r: LogicalRange, fraction: number): LogicalRange {
  const delta = (r.to - r.from) * fraction;
  return { from: r.from + delta, to: r.to + delta };
}

/** The first view of a series: the newest `bars` bars plus some room on the right. */
export function initialRange(total: number, bars = 150): LogicalRange {
  return { from: Math.max(0, total - bars), to: total + 4 };
}
