import { describe, expect, it } from "vitest";
import { parse } from "../commands";
import { dispatch } from "./dispatch";
import { createWorkspace } from "./workspace";

describe("dispatch", () => {
  it("RELIANCE opens a quote view, NIFTY an index view, in the focused panel", () => {
    const ws = createWorkspace({ layout: 2, focused: 1 });
    expect(dispatch(parse("reliance"), ws)).toBeUndefined();
    expect(ws.currentView(1)).toEqual({ kind: "quote", exchange: "NSE", symbol: "RELIANCE" });
    expect(dispatch(parse("nifty"), ws)).toBeUndefined();
    expect(ws.currentView(1)).toMatchObject({ kind: "index", title: "NIFTY 50" });
    expect(ws.currentView(0).kind).toBe("empty");
  });

  it("passes parse errors through without opening anything", () => {
    const ws = createWorkspace();
    expect(dispatch(parse("XX:ABC"), ws)).toMatch(/Unknown exchange/);
    expect(ws.currentView(0).kind).toBe("empty");
  });

  it("MKT opens the dashboard once, F2 does the same, and Esc goes back", () => {
    const ws = createWorkspace();
    expect(dispatch(parse("mkt"), ws)).toBeUndefined();
    expect(ws.currentView(0).kind).toBe("mkt");
    // Re-running it doesn't stack a second dashboard.
    dispatch(parse("MKT"), ws);
    expect(ws.back()).toBe(true);
    expect(ws.currentView(0).kind).toBe("empty");
  });

  it("a view can open another view in its own panel", () => {
    const ws = createWorkspace({ layout: 2, focused: 0 });
    ws.openIn(1, { kind: "mkt" });
    expect(ws.currentView(1).kind).toBe("mkt");
    expect(ws.currentView(0).kind).toBe("empty");
    ws.openIn(9, { kind: "mkt" }); // out of range: ignored
  });
});
