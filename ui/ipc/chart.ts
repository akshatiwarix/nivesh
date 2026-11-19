// Typed wrappers for the chart commands.
import { Channel } from "@tauri-apps/api/core";
import type { BarDto } from "./bindings/BarDto";
import type { HistoryDto } from "./bindings/HistoryDto";
import type { IndicatorPreviewDto } from "./bindings/IndicatorPreviewDto";
import type { IndicatorsDto } from "./bindings/IndicatorsDto";
import { call } from "./core";

/**
 * Candles for a chart as parallel columns. Omit `range` for the default for
 * the interval. Rejects with a message that says why if the public sources
 * can't serve that interval over that range.
 */
export function getHistory(
  exchange: string,
  symbol: string,
  interval: string,
  range?: string,
): Promise<HistoryDto> {
  return call<HistoryDto>("get_history", { exchange, symbol, interval, range });
}

/**
 * Streams the forming bar (built in Rust from the quote stream) until the
 * returned function is called.
 */
export async function subscribeFormingBar(
  exchange: string,
  symbol: string,
  interval: string,
  range: string | undefined,
  onBar: (bar: BarDto) => void,
): Promise<() => void> {
  const channel = new Channel<BarDto>();
  channel.onmessage = onBar;
  const id = await call<number>("subscribe_forming_bar", {
    exchange,
    symbol,
    interval,
    range,
    onBar: channel,
  });
  let done = false;
  return () => {
    if (done) return;
    done = true;
    void call<void>("unsubscribe", { id }).catch(() => undefined);
  };
}

/**
 * Indicator lines for the series the chart shows, computed in Rust. `specs`
 * are canonical texts such as `SMA 50`. The result carries the bar times it
 * was computed for.
 */
export function computeIndicators(
  exchange: string,
  symbol: string,
  interval: string,
  range: string | undefined,
  specs: readonly string[],
): Promise<IndicatorsDto> {
  return call<IndicatorsDto>("compute_indicators", { exchange, symbol, interval, range, specs });
}

/**
 * The indicators' newest values with the forming bar folded in, for moving
 * the series' last point while the bar is still forming.
 */
export function previewIndicators(
  exchange: string,
  symbol: string,
  interval: string,
  range: string | undefined,
  specs: readonly string[],
  bar: BarDto,
): Promise<IndicatorPreviewDto> {
  return call<IndicatorPreviewDto>("preview_indicators", {
    exchange,
    symbol,
    interval,
    range,
    specs,
    bar,
  });
}

/** The indicators saved for a symbol (they follow it across intervals). */
export function chartIndicators(exchange: string, symbol: string): Promise<string[]> {
  return call<string[]>("chart_indicators", { exchange, symbol });
}

export function saveChartIndicators(
  exchange: string,
  symbol: string,
  specs: readonly string[],
): Promise<void> {
  return call<void>("save_chart_indicators", { exchange, symbol, specs });
}
