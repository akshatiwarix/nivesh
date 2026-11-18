import { describe, expect, it } from "vitest";
import { createWorkspace } from "./workspace";

describe("workspace layout and focus", () => {
  it("starts with one focused panel", () => {
    const ws = createWorkspace();
    expect(ws.state).toEqual({ layout: 1, focused: 0 });
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
    expect(ws.state).toEqual({ layout: 2, focused: 0 });
    ws.focus(1);
    ws.setLayout(4);
    expect(ws.state).toEqual({ layout: 4, focused: 1 });
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
    expect(ws.state).toEqual({ layout: 1, focused: 0 });
  });

  it("a single panel never moves focus", () => {
    const ws = createWorkspace();
    ws.cycleFocus(1);
    ws.cycleFocus(-1);
    expect(ws.state.focused).toBe(0);
  });
});
