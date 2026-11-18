import { describe, expect, it } from "vitest";
import type { IndicatorDto, IndicatorLineDto } from "../../ipc";
import {
  histogramPoints,
  linePoints,
  paletteColor,
  removeSpecs,
  specText,
  splitByPlacement,
} from "./indicators";

const line = (values: (number | null)[], name = "SMA 3"): IndicatorLineDto => ({
  name,
  style: "line",
  values,
});

describe("indicator line shaping", () => {
  it("keeps one point per candle, with whitespace during warm-up", () => {
    const pts = linePoints([1000, 2000, 3000], line([null, null, 5]));
    expect(pts).toEqual([{ time: 1 }, { time: 2 }, { time: 3, value: 5 }]);
  });

  it("colours the histogram by sign", () => {
    const pts = histogramPoints([1000, 2000, 3000], line([null, 1, -1], "Hist"), {
      up: "G",
      down: "R",
    });
    expect(pts).toEqual([
      { time: 1 },
      { time: 2, value: 1, color: "G" },
      { time: 3, value: -1, color: "R" },
    ]);
  });

  it("cycles the palette and splits overlays from panes", () => {
    expect(paletteColor(["a", "b"], 3)).toBe("b");
    const mk = (placement: string, id: string): IndicatorDto => ({
      id,
      kind: id.split(" ")[0] ?? "",
      placement,
      lines: [],
      bounds: null,
      guides: [],
    });
    const { overlays, panes } = splitByPlacement([mk("overlay", "SMA 5"), mk("pane", "RSI 14")]);
    expect(overlays.map((i) => i.id)).toEqual(["SMA 5"]);
    expect(panes.map((i) => i.id)).toEqual(["RSI 14"]);
  });
});

describe("removing indicators", () => {
  const current = ["SMA 20", "SMA 50", "RSI 14"];

  it("RM RSI removes every indicator of that kind", () => {
    expect(removeSpecs(current, ["rsi"])).toEqual({ remaining: ["SMA 20", "SMA 50"] });
    expect(removeSpecs(current, ["SMA"])).toEqual({ remaining: ["RSI 14"] });
  });

  it("RM SMA 50 removes just that one", () => {
    expect(removeSpecs(current, ["sma", "50"])).toEqual({ remaining: ["SMA 20", "RSI 14"] });
  });

  it("says what is wrong", () => {
    expect(removeSpecs(current, [])).toEqual({ error: "RM needs an indicator, e.g. RM RSI" });
    expect(removeSpecs(current, ["MACD"])).toEqual({ error: "MACD is not on this chart" });
    expect(removeSpecs(current, ["SMA", "7"])).toEqual({ error: "SMA 7 is not on this chart" });
  });

  it("joins arguments into spec text", () => {
    expect(specText(["sma", "50"])).toBe("SMA 50");
  });
});
