// Typed wrappers for the chart commands.
import { Channel } from "@tauri-apps/api/core";
import type { BarDto } from "./bindings/BarDto";
import type { HistoryDto } from "./bindings/HistoryDto";
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
