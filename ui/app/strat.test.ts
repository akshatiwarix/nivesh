import { describe, expect, it } from "vitest";
import { parseStratArgs } from "./strat";

describe("parseStratArgs", () => {
  it("reads the positional numbers with the template's defaults", () => {
    expect(parseStratArgs(["MA", "10", "40", "3"])).toEqual({
      template: "MA",
      name: "MA 10/40",
      fast: 10,
      slow: 40,
      quantity: 3,
    });
    expect(parseStratArgs(["RSI"])).toMatchObject({ name: "RSI 14", fast: 14, quantity: 1 });
  });

  it("reads SL, T and RISK as percent or fraction, and names the variant", () => {
    expect(parseStratArgs(["BREAKOUT", "20", "SL", "2", "T", "0.05", "RISK", "1"])).toEqual({
      template: "BREAKOUT",
      name: "Breakout 20/10 SL2 T5 R1",
      fast: 20,
      slow: 10,
      quantity: 1,
      stopLoss: 0.02,
      target: 0.05,
      risk: 0.01,
    });
  });

  it("explains bad input", () => {
    expect(parseStratArgs([])).toMatch(/Which template/);
    expect(parseStratArgs(["MACD"])).toMatch(/Which template/);
    expect(parseStratArgs(["MA", "x"])).toMatch(/not a positive number/);
    expect(parseStratArgs(["MA", "SL"])).toMatch(/SL needs a positive percentage/);
    expect(parseStratArgs(["MA", "20", "STOP", "2"])).toMatch(/not a positive number/);
  });
});
