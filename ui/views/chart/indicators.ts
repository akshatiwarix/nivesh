// Turns the backend's indicator lines into library series. Pure data shaping
// lives here (and is tested); ChartView owns the chart handle.

import type { IndicatorDto, IndicatorLineDto } from "../../ipc";

export interface LinePoint {
  time: number;
  value?: number;
}

/**
 * Points for one line, one per candle. Bars where the indicator isn't defined
 * yet (warm-up) become whitespace, so the line lines up with the candles.
 */
export function linePoints(t: readonly number[], line: IndicatorLineDto): LinePoint[] {
  const out: LinePoint[] = new Array(t.length);
  for (let i = 0; i < t.length; i++) {
    const value = line.values[i];
    const time = (t[i] as number) / 1000;
    out[i] = value === null || value === undefined ? { time } : { time, value };
  }
  return out;
}

export interface HistogramPoint extends LinePoint {
  color?: string;
}

/** MACD-style histogram: one colour above zero, another below. */
export function histogramPoints(
  t: readonly number[],
  line: IndicatorLineDto,
  colors: { up: string; down: string },
): HistogramPoint[] {
  return linePoints(t, line).map((p) =>
    p.value === undefined ? p : { ...p, color: p.value >= 0 ? colors.up : colors.down },
  );
}

/** Palette index for the n-th overlay, cycling. */
export function paletteColor(palette: readonly string[], n: number): string {
  return palette[n % palette.length] ?? "#ffb000";
}

/** Splits indicators into those drawn on the price pane and those in their own. */
export function splitByPlacement(all: readonly IndicatorDto[]) {
  return {
    overlays: all.filter((i) => i.placement === "overlay"),
    panes: all.filter((i) => i.placement === "pane"),
  };
}

/** The spec text a command's arguments name: `["SMA", "50"]` -> `SMA 50`. */
export function specText(args: readonly string[]): string {
  return args.join(" ").trim().toUpperCase();
}

export type RemoveResult = { remaining: string[] } | { error: string };

/**
 * `RM RSI` removes every RSI; `RM SMA 50` removes just that one. Names are
 * matched on the canonical id, so `RM SMA 50` and `RM sma 50` agree.
 */
export function removeSpecs(current: readonly string[], args: readonly string[]): RemoveResult {
  const text = specText(args);
  if (text === "") return { error: "RM needs an indicator, e.g. RM RSI" };
  const [kind] = text.split(/\s+/);
  const exact = args.length > 1;
  const remaining = current.filter((id) =>
    exact ? id.toUpperCase() !== text : id.split(" ")[0] !== kind,
  );
  if (remaining.length === current.length) return { error: `${text} is not on this chart` };
  return { remaining };
}
