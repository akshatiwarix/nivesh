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
});
