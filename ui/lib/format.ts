// Number and time formatting for market data. Display only: values stay f64
// in the data layer. Indian digit grouping (1,23,456.00) throughout.

const NO_VALUE = "—";

const formatters = new Map<number, Intl.NumberFormat>();
function nf(decimals: number): Intl.NumberFormat {
  let f = formatters.get(decimals);
  if (!f) {
    f = new Intl.NumberFormat("en-IN", {
      minimumFractionDigits: decimals,
      maximumFractionDigits: decimals,
    });
    formatters.set(decimals, f);
  }
  return f;
}

const isNum = (v: number | null | undefined): v is number =>
  typeof v === "number" && Number.isFinite(v);

/** `1,23,456.50`, or an em dash for missing values. */
export function formatPrice(v: number | null | undefined, decimals = 2): string {
  return isNum(v) ? nf(decimals).format(v) : NO_VALUE;
}

/** `+12.50`, `-3.00`, `0.00`. */
export function formatChange(v: number | null | undefined, decimals = 2): string {
  if (!isNum(v)) return NO_VALUE;
  const sign = v > 0 ? "+" : "";
  return `${sign}${nf(decimals).format(v)}`;
}

/** `+0.62%`, `-1.04%`. */
export function formatPct(v: number | null | undefined): string {
  if (!isNum(v)) return NO_VALUE;
  return `${v > 0 ? "+" : ""}${nf(2).format(v)}%`;
}

/** Whole numbers with Indian grouping: `1,23,45,678`. */
export function formatCount(v: number | null | undefined): string {
  return isNum(v) ? nf(0).format(Math.round(v)) : NO_VALUE;
}

/** Colour for a change. Always pair it with {@link arrow} (colour-blind safe). */
export function changeTone(v: number | null | undefined): "up" | "down" | undefined {
  if (!isNum(v) || v === 0) return undefined;
  return v > 0 ? "up" : "down";
}

export function arrow(v: number | null | undefined): string {
  if (!isNum(v) || v === 0) return "–";
  return v > 0 ? "▲" : "▼";
}

/** `▲ +12.50` */
export function formatMove(v: number | null | undefined): string {
  return isNum(v) ? `${arrow(v)} ${formatChange(v)}` : NO_VALUE;
}

/** `▲ +0.62%` */
export function formatMovePct(v: number | null | undefined): string {
  return isNum(v) ? `${arrow(v)} ${formatPct(v)}` : NO_VALUE;
}

/** `52,300.00 – 77,100.00`, or a dash if either end is missing. */
export function formatRange(low: number | null | undefined, high: number | null | undefined) {
  return isNum(low) && isNum(high) ? `${formatPrice(low)} – ${formatPrice(high)}` : NO_VALUE;
}

/**
 * Where `value` sits between `low` and `high`, as 0..1. Used for the 52-week
 * position marker. Undefined if the range is missing or empty.
 */
export function rangePosition(
  value: number | null | undefined,
  low: number | null | undefined,
  high: number | null | undefined,
): number | undefined {
  if (!isNum(value) || !isNum(low) || !isNum(high) || high <= low) return undefined;
  return Math.min(1, Math.max(0, (value - low) / (high - low)));
}
