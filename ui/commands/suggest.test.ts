import { describe, expect, it } from "vitest";
import type { InstrumentDto } from "../ipc";
import { instrumentLine, searchTarget } from "./suggest";

const reliance: InstrumentDto = {
  exchange: "NSE",
  symbol: "RELIANCE",
  name: "Reliance Industries Limited",
  series: "EQ",
};

describe("searchTarget", () => {
  it("searches for plain text, including names with spaces", () => {
    expect(searchTarget("relia")?.query).toBe("relia");
    expect(searchTarget("  tata mot  ")?.query).toBe("tata mot");
  });

  it("leaves commands and explicit exchanges alone", () => {
    for (const text of ["", "   ", "HELP", "mkt", "help me", "BSE:REL", "nse:", "?"]) {
      expect(searchTarget(text), text).toBeUndefined();
    }
  });

  it("does not search absurdly long text", () => {
    expect(searchTarget("A".repeat(65))).toBeUndefined();
    expect(searchTarget("A".repeat(64))).toBeDefined();
  });

  it("accepting a suggestion runs the explicit-exchange form", () => {
    expect(instrumentLine(reliance)).toBe("NSE:RELIANCE");
    expect(searchTarget("relia")?.line(reliance)).toBe("NSE:RELIANCE");
  });
});
