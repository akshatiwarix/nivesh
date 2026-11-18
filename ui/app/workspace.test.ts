import { describe, expect, it } from "vitest";
import { createWorkspace } from "./workspace";

describe("workspace layout and focus", () => {
  it("starts with one focused panel", () => {
    const ws = createWorkspace();
    expect(ws.state).toMatchObject({ layout: 1, focused: 0 });
  });

  it("cycles focus within the visible panels, wrapping both ways", () => {
    const ws = createWorkspace({ layout: 4 });
    ws.cycleFocus(1);
    ws.cycleFocus(1);
    ws.cycleFocus(1);
    expect(ws.state.focused).toBe(3);
    ws.cycleFocus(1);
    expect(ws.state.focused).toBe(0);
    ws.cycleFocus(-1);
    expect(ws.state.focused).toBe(3);
  });

  it("keeps focus on a visible panel when the layout shrinks", () => {
    const ws = createWorkspace({ layout: 4, focused: 3 });
    ws.setLayout(2);
    expect(ws.state).toMatchObject({ layout: 2, focused: 0 });
    ws.focus(1);
    ws.setLayout(4);
    expect(ws.state).toMatchObject({ layout: 4, focused: 1 });
  });

  it("ignores focus requests for hidden or invalid panels", () => {
    const ws = createWorkspace({ layout: 2 });
    ws.focus(3);
    ws.focus(-1);
    ws.focus(0.5);
    expect(ws.state.focused).toBe(0);
  });

  it("sanitises an invalid initial state", () => {
    // e.g. a corrupted persisted layout
    const ws = createWorkspace({ layout: 3 as never, focused: 7 });
    expect(ws.state).toMatchObject({ layout: 1, focused: 0 });
  });

  it("a single panel never moves focus", () => {
    const ws = createWorkspace();
    ws.cycleFocus(1);
    ws.cycleFocus(-1);
    expect(ws.state.focused).toBe(0);
  });
});

describe("per-panel view stacks", () => {
  const quote = (symbol: string) => ({ kind: "quote", exchange: "NSE", symbol }) as const;

  it("opens views in the focused panel only, and Esc pops them", () => {
    const ws = createWorkspace({ layout: 2 });
    ws.open(quote("RELIANCE"));
    ws.focus(1);
    expect(ws.currentView(0)).toEqual(quote("RELIANCE"));
    expect(ws.currentView(1).kind).toBe("empty");
    ws.open({ kind: "index", id: "NIFTY50", title: "NIFTY 50" });
    ws.open(quote("TCS"));
    expect(ws.back()).toBe(true);
    expect(ws.currentView(1).kind).toBe("index");
    expect(ws.back()).toBe(true);
    expect(ws.currentView(1).kind).toBe("empty");
    expect(ws.back()).toBe(false);
    expect(ws.currentView(0)).toEqual(quote("RELIANCE"));
  });

  it("bounds the stack", () => {
    const ws = createWorkspace();
    for (let i = 0; i < 50; i++) ws.open(quote(`S${i}`));
    expect(ws.state.stacks[0]?.length).toBe(20);
    expect(ws.currentView(0)).toEqual(quote("S49"));
  });

  it("keeps hidden panels' stacks when the layout shrinks", () => {
    const ws = createWorkspace({ layout: 4 });
    ws.focus(3);
    ws.open(quote("INFY"));
    ws.setLayout(1);
    ws.setLayout(4);
    expect(ws.currentView(3)).toEqual(quote("INFY"));
  });
});
