import { describe, expect, it, vi } from "vitest";
import { createHistory, MAX_HISTORY } from "./history";

describe("history", () => {
  it("walks back and forward and restores the draft", () => {
    const h = createHistory(["A", "B", "C"]);
    expect(h.prev("dra")).toBe("C");
    expect(h.prev("C")).toBe("B");
    expect(h.prev("B")).toBe("A");
    expect(h.prev("A")).toBeUndefined();
    expect(h.next()).toBe("B");
    expect(h.next()).toBe("C");
    expect(h.next()).toBe("dra");
    expect(h.next()).toBeUndefined();
  });

  it("push skips blanks and consecutive duplicates, and notifies", () => {
    const onChange = vi.fn();
    const h = createHistory([], onChange);
    h.push("  ");
    h.push("NIFTY");
    h.push("NIFTY");
    h.push("HELP");
    expect(h.entries()).toEqual(["NIFTY", "HELP"]);
    expect(onChange).toHaveBeenCalledTimes(2);
  });

  it("is bounded", () => {
    const h = createHistory();
    for (let i = 0; i < MAX_HISTORY + 10; i++) h.push(`S${i}`);
    expect(h.entries().length).toBe(MAX_HISTORY);
    expect(h.entries()[MAX_HISTORY - 1]).toBe(`S${MAX_HISTORY + 9}`);
  });

  it("empty history navigates nowhere; push resets position", () => {
    const h = createHistory();
    expect(h.prev("x")).toBeUndefined();
    h.push("A");
    h.prev("");
    h.push("B");
    expect(h.prev("")).toBe("B");
  });
});
