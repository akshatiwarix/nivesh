import { beforeEach, describe, expect, it, vi } from "vitest";

const ipc = vi.hoisted(() => ({
  riskSetLimit: vi.fn(),
  riskClearLimits: vi.fn(),
  liveSwitch: vi.fn(),
  strategyArm: vi.fn(),
  strategyDisarm: vi.fn(async () => true),
}));

vi.mock("../ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../ipc")>()),
  inTauri: () => true,
  ...ipc,
}));

import { parse } from "../commands";
import { answerChallenge, cancelChallenge, challenge } from "../data/risk";
import { dispatch } from "./dispatch";
import { createWorkspace } from "./workspace";

const done = { done: true, confirm: null };
const needs = (phrase: string) => ({ done: false, confirm: phrase });

describe("RISK", () => {
  beforeEach(() => {
    for (const f of Object.values(ipc)) f.mockReset();
    cancelChallenge();
  });

  it("bare RISK opens the view", async () => {
    const ws = createWorkspace();
    expect(await dispatch(parse("risk"), ws)).toBeUndefined();
    expect(ws.currentView(0).kind).toBe("risk");
  });

  it("a tightening applies at once", async () => {
    ipc.riskSetLimit.mockResolvedValue(done);
    const ws = createWorkspace();
    expect(await dispatch(parse("risk set loss 2,000 for trend"), ws)).toBeUndefined();
    expect(ipc.riskSetLimit).toHaveBeenCalledWith("LOSS", 2000, "TREND", undefined);
    expect(challenge()).toBeUndefined();
  });

  it("a loosening waits for the typed phrase, then sends it", async () => {
    ipc.riskSetLimit.mockResolvedValueOnce(needs("LOOSEN LIMITS"));
    const ws = createWorkspace();
    const msg = await dispatch(parse("risk set qty 5000"), ws);
    expect(msg).toContain("type LOOSEN LIMITS");
    expect(challenge()?.phrase).toBe("LOOSEN LIMITS");
    expect(ws.currentView(0).kind).toBe("risk");

    ipc.riskSetLimit.mockResolvedValueOnce(needs("LOOSEN LIMITS"));
    expect(await answerChallenge("LOSEN LIMITS")).toContain("did not match");
    expect(challenge()).toBeDefined();

    ipc.riskSetLimit.mockResolvedValueOnce(done);
    expect(await answerChallenge(" LOOSEN LIMITS ")).toBeUndefined();
    expect(ipc.riskSetLimit).toHaveBeenLastCalledWith("QTY", 5000, undefined, "LOOSEN LIMITS");
    expect(challenge()).toBeUndefined();
  });

  it("rejects malformed input before calling the backend", async () => {
    const ws = createWorkspace();
    expect(await dispatch(parse("risk set loss lots"), ws)).toContain("RISK SET");
    expect(await dispatch(parse("risk set loss 10 trend"), ws)).toContain("RISK SET");
    expect(await dispatch(parse("risk clear"), ws)).toContain("RISK CLEAR");
    expect(ipc.riskSetLimit).not.toHaveBeenCalled();
  });

  it("RISK CLEAR asks for the phrase", async () => {
    ipc.riskClearLimits.mockResolvedValueOnce(needs("LOOSEN LIMITS"));
    const ws = createWorkspace();
    expect(await dispatch(parse("risk clear trend"), ws)).toContain("LOOSEN LIMITS");
    expect(ipc.riskClearLimits).toHaveBeenCalledWith("TREND", undefined);
  });
});

describe("LIVE, ARM and DISARM", () => {
  beforeEach(() => {
    for (const f of Object.values(ipc)) f.mockReset();
    ipc.strategyDisarm.mockResolvedValue(true);
    cancelChallenge();
  });

  it("LIVE ON asks for the phrase; LIVE OFF is immediate", async () => {
    ipc.liveSwitch.mockResolvedValueOnce(needs("ENABLE LIVE TRADING"));
    const ws = createWorkspace();
    expect(await dispatch(parse("live on"), ws)).toContain("ENABLE LIVE TRADING");
    expect(ipc.liveSwitch).toHaveBeenCalledWith(true, undefined);

    ipc.liveSwitch.mockResolvedValueOnce(done);
    expect(await dispatch(parse("live off"), ws)).toBeUndefined();
    expect(ipc.liveSwitch).toHaveBeenLastCalledWith(false, undefined);
    // A safer action clears a pending one.
    expect(challenge()).toBeUndefined();
    expect(await dispatch(parse("live"), ws)).toBe("LIVE ON or LIVE OFF.");
  });

  it("ARM shows the backend's refusal, or asks for ARM <NAME> LIVE", async () => {
    const ws = createWorkspace();
    ipc.strategyArm.mockRejectedValueOnce(new Error("no broker that can trade is connected"));
    expect(await dispatch(parse("arm trend"), ws)).toBe("no broker that can trade is connected");
    expect(challenge()).toBeUndefined();

    ipc.strategyArm.mockResolvedValueOnce(needs("ARM TREND LIVE"));
    expect(await dispatch(parse("arm trend"), ws)).toContain("ARM TREND LIVE");
    expect(challenge()?.action).toBe("arm TREND for live trading");
    expect(await dispatch(parse("arm"), ws)).toContain("which strategy");
  });

  it("DISARM is immediate", async () => {
    const ws = createWorkspace();
    expect(await dispatch(parse("disarm trend"), ws)).toBeUndefined();
    expect(ipc.strategyDisarm).toHaveBeenCalledWith("TREND");
  });
});
