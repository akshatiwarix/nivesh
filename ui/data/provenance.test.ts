import { describe, expect, it } from "vitest";
import { prov } from "./fixtures";
import { summarizeProvenance } from "./provenance";

describe("summarizeProvenance", () => {
  it("is undefined when there is nothing to summarise", () => {
    expect(summarizeProvenance(null)).toBeUndefined();
    expect(summarizeProvenance(undefined)).toBeUndefined();
    expect(summarizeProvenance([])).toBeUndefined();
  });

  it("passes a single provenance through", () => {
    expect(summarizeProvenance(prov("nse"))).toEqual({
      freshness: "DELAYED",
      providers: ["nse"],
      oldestAsOfMs: prov().asOfMs,
    });
  });

  it("takes the worst freshness, in order STALE > DELAYED > EOD > LIVE", () => {
    const f = (...fs: Parameters<typeof prov>[1][]) =>
      summarizeProvenance(fs.map((x) => prov("nse", x)))?.freshness;
    expect(f("LIVE", "EOD")).toBe("EOD");
    expect(f("EOD", "DELAYED")).toBe("DELAYED");
    expect(f("DELAYED", "STALE", "LIVE")).toBe("STALE");
    expect(f("LIVE")).toBe("LIVE");
  });

  it("lists distinct providers in first-seen order and the oldest time", () => {
    const s = summarizeProvenance([
      { ...prov("yahoo"), asOfMs: 500 },
      prov("nse"),
      { ...prov("yahoo"), asOfMs: 900 },
    ]);
    expect(s?.providers).toEqual(["yahoo", "nse"]);
    expect(s?.oldestAsOfMs).toBe(500);
  });
});
