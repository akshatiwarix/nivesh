import { describe, expect, it } from "vitest";
import { applyTab, completions } from "./complete";

const WORDS = ["BANKNIFTY", "HELP", "NIFTY", "NIFTY50", "SENSEX"];

describe("completions", () => {
  it.each([
    ["ni", ["NIFTY", "NIFTY50"]],
    ["NIFTY", ["NIFTY50"]],
    ["h", ["HELP"]],
    ["zz", []],
    ["", []],
    ["BSE:NI", []],
    ["NIFTY C", []],
  ])("%j", (input, expected) => {
    expect(completions(input, WORDS).candidates).toEqual(expected);
  });
});

describe("applyTab", () => {
  it("completes a unique match and appends a space", () => {
    expect(applyTab("he", WORDS).text).toBe("HELP ");
  });

  it("cycles through ambiguous matches", () => {
    const a = applyTab("ni", WORDS);
    expect(a.text).toBe("NIFTY");
    const b = applyTab(a.text, WORDS, a.cycle);
    expect(b.text).toBe("NIFTY50");
    const c = applyTab(b.text, WORDS, b.cycle);
    expect(c.text).toBe("NIFTY");
  });

  it("leaves text alone when nothing matches", () => {
    expect(applyTab("zz", WORDS).text).toBe("zz");
  });
});
