import { describe, expect, it } from "vitest";
import { drawdownPoints, equityPoints, splitInstrument, tradeMarkers } from "./btChart";

const colors = { buy: "b", win: "w", loss: "l" };
const trade = (entryT: number, exitT: number, pnl: number) => ({
  entryPrice: 100,
  exitPrice: 101,
  quantity: 2,
  charges: 0.5,
  pnl,
  entryT,
  exitT,
});

describe("backtest chart shaping", () => {
  it("puts an entry and an exit marker at the trade-log times, in time order", () => {
    const markers = tradeMarkers([trade(5_000, 9_000, -3), trade(1_000, 3_000, 4)], colors);
    expect(markers.map((m) => [m.time, m.shape, m.color])).toEqual([
      [1, "arrowUp", "b"],
      [3, "arrowDown", "w"],
      [5, "arrowUp", "b"],
      [9, "arrowDown", "l"],
    ]);
    expect(markers[0]?.text).toBe("B 2");
  });

  it("turns the curve into equity and below-zero drawdown points", () => {
    const curve = [
      { t: 1_000, equity: 100_000, drawdown: 0 },
      { t: 2_000, equity: 99_000, drawdown: 1_000 },
    ];
    expect(equityPoints(curve)).toEqual([
      { time: 1, value: 100_000 },
      { time: 2, value: 99_000 },
    ]);
    expect(drawdownPoints(curve)).toEqual([
      { time: 1, value: 0 },
      { time: 2, value: -1_000 },
    ]);
  });

  it("splits an instrument id", () => {
    expect(splitInstrument("BSE:TCS")).toEqual({ exchange: "BSE", symbol: "TCS" });
    expect(splitInstrument("TCS")).toEqual({ exchange: "NSE", symbol: "TCS" });
  });
});
