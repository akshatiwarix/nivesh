import { beforeEach, describe, expect, it, vi } from "vitest";

const ipc = vi.hoisted(() => ({
  paperStart: vi.fn(async () => ({})),
  paperStop: vi.fn(async () => undefined),
  backtestStrategy: vi.fn(async () => ({})),
}));

vi.mock("../ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../ipc")>()),
  inTauri: () => true,
  paperStart: ipc.paperStart,
  paperStop: ipc.paperStop,
  backtestStrategy: ipc.backtestStrategy,
}));

import { parse } from "../commands";
import { dispatch } from "./dispatch";
import { createWorkspace } from "./workspace";

describe("PAPER", () => {
  beforeEach(() => {
    ipc.paperStart.mockClear();
    ipc.paperStop.mockClear();
  });

  it("PAPER START <NAME> passes the stream when given, and opens the dashboard", async () => {
    const ws = createWorkspace();
    expect(await dispatch(parse("paper start trend nse:tcs 5m"), ws)).toBeUndefined();
    expect(ipc.paperStart).toHaveBeenCalledWith("TREND", "TCS", "5M");
    expect(ws.currentView(0).kind).toBe("paper");

    await dispatch(parse("paper start trend"), ws);
    expect(ipc.paperStart).toHaveBeenLastCalledWith("TREND", undefined, undefined);
  });

  it("PAPER STOP <NAME> stops it", async () => {
    const ws = createWorkspace();
    await dispatch(parse("paper stop trend"), ws);
    expect(ipc.paperStop).toHaveBeenCalledWith("TREND");
    expect(ipc.paperStart).not.toHaveBeenCalled();
  });
});

describe("BT", () => {
  beforeEach(() => ipc.backtestStrategy.mockClear());

  it("BT <NAME> <SYMBOL> [INTERVAL] [RANGE] backtests on that symbol", async () => {
    const ws = createWorkspace();
    await dispatch(parse("bt trend tcs 1d 5y"), ws);
    expect(ipc.backtestStrategy).toHaveBeenCalledWith("TREND", "1D", "5Y", "TCS");
    expect(ws.currentView(0)).toEqual({ kind: "bt", name: "TREND" });
  });

  it("the older BT <NAME> [INTERVAL] [RANGE] form still works", async () => {
    const ws = createWorkspace();
    await dispatch(parse("bt trend 1w 2y"), ws);
    expect(ipc.backtestStrategy).toHaveBeenCalledWith("TREND", "1W", "2Y", undefined);
    await dispatch(parse("bt trend"), ws);
    expect(ipc.backtestStrategy).toHaveBeenLastCalledWith("TREND", "1D", "1Y", undefined);
  });
});
