import { describe, expect, it } from "vitest";
import { parse } from "./parser";
import { COMMANDS, commandForFunctionKey, completionWords, findFunction } from "./registry";

describe("parse", () => {
  it.each([
    ["", { kind: "empty" }],
    ["   ", { kind: "empty" }],
    ["help", { kind: "command", id: "help", args: [] }],
    ["  HELP  ", { kind: "command", id: "help", args: [] }],
    ["?", { kind: "command", id: "help", args: [] }],
    ["RELIANCE", { kind: "instrument", exchange: "NSE", symbol: "RELIANCE", args: [] }],
    ["reliance", { kind: "instrument", exchange: "NSE", symbol: "RELIANCE", args: [] }],
    ["BSE:RELIANCE", { kind: "instrument", exchange: "BSE", symbol: "RELIANCE", args: [] }],
    ["bse:500325", { kind: "instrument", exchange: "BSE", symbol: "500325", args: [] }],
    ["nse:m&m", { kind: "instrument", exchange: "NSE", symbol: "M&M", args: [] }],
    ["BAJAJ-AUTO", { kind: "instrument", exchange: "NSE", symbol: "BAJAJ-AUTO", args: [] }],
    ["NIFTY", { kind: "index", id: "NIFTY50", title: "NIFTY 50", args: [] }],
    ["banknifty", { kind: "index", id: "BANKNIFTY", title: "NIFTY BANK", args: [] }],
    ["Sensex", { kind: "index", id: "SENSEX", title: "SENSEX", args: [] }],
  ])("%j", (input, expected) => {
    expect(parse(input)).toEqual(expected);
  });

  it.each([
    ["XX:RELIANCE", /Unknown exchange "XX"/],
    ["NSE:", /Missing symbol/],
    ["-RELIANCE", /Invalid symbol/],
    ["REL$IANCE", /Invalid symbol/],
    ["A".repeat(25), /Invalid symbol/],
    ["RELIANCE ZZZ", /Unknown function "ZZZ"/],
    ["NIFTY ZZZ", /Unknown function "ZZZ"/],
  ])("rejects %j", (input, message) => {
    const p = parse(input);
    expect(p.kind).toBe("error");
    expect(p.kind === "error" && p.message).toMatch(message);
  });

  it("index aliases win over stock symbols, commands win over both", () => {
    // `NSE:NIFTY` is explicit, so it is the stock, not the index.
    expect(parse("NSE:NIFTY").kind).toBe("instrument");
    expect(parse("NIFTY").kind).toBe("index");
    expect(parse("HELP").kind).toBe("command");
  });

  it("passes extra arguments to commands", () => {
    expect(parse("help quote")).toEqual({ kind: "command", id: "help", args: ["QUOTE"] });
  });
});

describe("registry", () => {
  it("has unique command ids and names", () => {
    const ids = COMMANDS.map((c) => c.id);
    const names = COMMANDS.flatMap((c) => c.names);
    expect(new Set(ids).size).toBe(ids.length);
    expect(new Set(names).size).toBe(names.length);
  });

  it("maps function keys to commands", () => {
    expect(commandForFunctionKey(1)?.id).toBe("help");
    expect(commandForFunctionKey(9)).toBeUndefined();
  });

  it("offers sorted completion words without punctuation-only names", () => {
    const words = completionWords();
    expect(words).toContain("HELP");
    expect(words).toContain("NIFTY");
    expect(words).not.toContain("?");
    expect([...words].sort()).toEqual(words);
  });

  it("finds no functions yet", () => {
    expect(findFunction("CH")).toBeUndefined();
  });
});
