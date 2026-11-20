import { render } from "solid-js/web";
import { beforeEach, describe, expect, it, vi } from "vitest";

const engaged = (over = {}) => ({
  engaged: true,
  reason: "manual",
  detail: "engaged by the user",
  engagedAt: 0,
  resettableAt: 0,
  resetPhrase: "RESET KILL SWITCH",
  ...over,
});

const ipc = vi.hoisted(() => ({
  killEngage: vi.fn(),
  killStatus: vi.fn(),
  killReset: vi.fn(),
}));

vi.mock("../ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../ipc")>()),
  inTauri: () => true,
  ...ipc,
}));

import { parse } from "../commands";
import { answerChallenge, cancelChallenge, challenge, kill, setKill } from "../data/risk";
import { resolveKey } from "../keys/keymap";
import { runAction } from "./actions";
import { dispatch } from "./dispatch";
import { KillControl, LiveBanner } from "./SafetyBar";
import { createWorkspace } from "./workspace";

const tick = () => new Promise<void>((r) => setTimeout(r, 0));

describe("the kill switch", () => {
  beforeEach(() => {
    for (const f of Object.values(ipc)) f.mockReset();
    ipc.killEngage.mockResolvedValue(engaged());
    setKill(undefined);
    cancelChallenge();
  });

  it("Ctrl+Shift+K engages it, even while typing", async () => {
    const binding = resolveKey(
      { key: "K", code: "KeyK", ctrlKey: true, shiftKey: true, altKey: false, metaKey: false },
      true,
    );
    expect(binding?.action).toEqual({ type: "kill" });
    runAction({ type: "kill" }, createWorkspace());
    await tick();
    expect(ipc.killEngage).toHaveBeenCalledOnce();
    expect(kill()?.engaged).toBe(true);
  });

  it("KILL engages it with no confirmation", async () => {
    expect(await dispatch(parse("kill"), createWorkspace())).toBeUndefined();
    expect(ipc.killEngage).toHaveBeenCalledOnce();
  });

  it("KILL RESET waits out the cool-down, then asks for the phrase", async () => {
    const ws = createWorkspace();
    ipc.killStatus.mockResolvedValueOnce(engaged({ resettableAt: Date.now() + 120_000 }));
    expect(await dispatch(parse("kill reset"), ws)).toContain("cool-down");
    expect(challenge()).toBeUndefined();

    ipc.killStatus.mockResolvedValueOnce(engaged());
    expect(await dispatch(parse("kill reset"), ws)).toContain("RESET KILL SWITCH");
    expect(ws.currentView(0).kind).toBe("risk");
    expect(await answerChallenge("RESET KILL")).toContain("did not match");
    expect(ipc.killReset).not.toHaveBeenCalled();

    ipc.killReset.mockResolvedValueOnce(engaged({ engaged: false }));
    expect(await answerChallenge("RESET KILL SWITCH")).toBeUndefined();
    expect(ipc.killReset).toHaveBeenCalledWith("RESET KILL SWITCH");
    expect(kill()?.engaged).toBe(false);
  });

  it("KILL RESET when it is off says so", async () => {
    ipc.killStatus.mockResolvedValueOnce(engaged({ engaged: false }));
    expect(await dispatch(parse("kill reset"), createWorkspace())).toContain("not engaged");
  });

  it("the status-bar control engages on click and shows the engaged state", async () => {
    const root = document.createElement("div");
    // Solid delegates clicks from the document.
    document.body.append(root);
    const dispose = render(() => <KillControl />, root);
    const button = root.querySelector("button");
    expect(button?.textContent).toContain("KILL");
    button?.click();
    await tick();
    expect(ipc.killEngage).toHaveBeenCalledOnce();
    expect(root.textContent).toContain("KILL SWITCH ENGAGED");
    dispose();
    root.remove();
  });
});

describe("LiveBanner", () => {
  it("is empty in paper, red and named when armed", () => {
    const root = document.createElement("div");
    const dispose = render(() => <LiveBanner liveEnabled={false} armed={[]} />, root);
    expect(root.querySelector(".live-banner")).toBeNull();
    dispose();

    const armed = [{ name: "TREND", until: Date.UTC(2026, 10, 20, 10, 0) }];
    const dispose2 = render(() => <LiveBanner liveEnabled={true} armed={armed} />, root);
    expect(root.textContent).toContain("LIVE");
    expect(root.textContent).toContain("TREND armed until 15:30 IST");
    dispose2();
  });
});
