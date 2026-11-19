// Shapes a backtest result for the chart: trade markers on the candles,
// and the equity and drawdown curves (computed in Rust) as series points.

import type { BacktestTradeDto, CurvePointDto } from "../../ipc";

export interface Marker {
  /** UTC seconds, as the chart library wants. */
  time: number;
  position: "belowBar" | "aboveBar";
  shape: "arrowUp" | "arrowDown";
  color: string;
  text: string;
}

export interface ValuePoint {
  time: number;
  value: number;
}

export interface MarkerColors {
  buy: string;
  win: string;
  loss: string;
}

/**
 * An arrow under the bar each trade entered on, and one over the bar it
 * left on, coloured by the trade's net result. Times are the fill bars'
 * times, the same ones the trade list shows.
 */
export function tradeMarkers(trades: readonly BacktestTradeDto[], colors: MarkerColors): Marker[] {
  const out: Marker[] = [];
  for (const t of trades) {
    out.push({
      time: t.entryT / 1000,
      position: "belowBar",
      shape: "arrowUp",
      color: colors.buy,
      text: `B ${t.quantity}`,
    });
    out.push({
      time: t.exitT / 1000,
      position: "aboveBar",
      shape: "arrowDown",
      color: t.pnl >= 0 ? colors.win : colors.loss,
      text: "S",
    });
  }
  // The library wants markers in time order.
  return out.sort((a, b) => a.time - b.time);
}

export function equityPoints(curve: readonly CurvePointDto[]): ValuePoint[] {
  return curve.map((p) => ({ time: p.t / 1000, value: p.equity }));
}

/** Drawdown below zero, so the pane reads as distance under the peak. */
export function drawdownPoints(curve: readonly CurvePointDto[]): ValuePoint[] {
  return curve.map((p) => ({ time: p.t / 1000, value: p.drawdown === 0 ? 0 : -p.drawdown }));
}

/** `NSE:TCS` into its exchange and symbol. */
export function splitInstrument(id: string): { exchange: string; symbol: string } {
  const [exchange, symbol] = id.includes(":") ? id.split(":", 2) : ["NSE", id];
  return { exchange: exchange ?? "NSE", symbol: symbol ?? id };
}
