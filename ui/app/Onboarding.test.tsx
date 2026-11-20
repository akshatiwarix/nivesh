import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";
import { parse } from "../commands";
import { DISCLAIMER, isLiveAction } from "../data/disclaimer";
import { finishOnboarding } from "../ipc";
import { dispatch } from "./dispatch";
import { Onboarding } from "./Onboarding";
import { closeOnboarding, onboardingOpen, showOnboarding } from "./onboardingState";
import { createWorkspace } from "./workspace";

vi.mock("../ipc", async (actual) => ({
  ...(await actual<typeof import("../ipc")>()),
  inTauri: () => true,
  finishOnboarding: vi.fn(() => Promise.resolve()),
}));

const tick = () => new Promise<void>((r) => setTimeout(r, 0));

describe("onboarding", () => {
  let dispose: (() => void) | undefined;
  let root: HTMLElement;

  function mount() {
    root = document.createElement("div");
    document.body.append(root);
    dispose = render(() => <Onboarding />, root);
  }

  afterEach(() => {
    closeOnboarding();
    dispose?.();
    root?.remove();
    vi.mocked(finishOnboarding).mockClear();
  });

  const key = (k: string) =>
    root
      .querySelector("[role=dialog]")
      ?.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true }));

  it("explains DELAYED data, paper by default and the disclaimer", async () => {
    mount();
    showOnboarding(true);
    await tick();
    const text = root.textContent ?? "";
    expect(text).toContain("DELAYED");
    expect(text).toContain("paper by default");
    expect(text).toContain(DISCLAIMER);
    expect(document.activeElement?.textContent).toBe("I understand");
  });

  it("on first run only the button closes it, and that is recorded", async () => {
    mount();
    showOnboarding(true);
    await tick();
    key("Escape");
    expect(onboardingOpen()).toBe("first-run");
    (document.activeElement as HTMLButtonElement).click();
    expect(onboardingOpen()).toBeUndefined();
    expect(finishOnboarding).toHaveBeenCalledOnce();
  });

  it("WELCOME shows it again; Esc closes it and records nothing", async () => {
    mount();
    expect(await dispatch(parse("welcome"), createWorkspace())).toBeUndefined();
    await tick();
    expect(onboardingOpen()).toBe("again");
    key("Escape");
    expect(onboardingOpen()).toBeUndefined();
    expect(finishOnboarding).not.toHaveBeenCalled();
  });

  it("SETTINGS opens the settings view", async () => {
    const ws = createWorkspace();
    expect(await dispatch(parse("settings"), ws)).toBeUndefined();
    expect(ws.currentView(0)).toEqual({ kind: "settings" });
  });

  it("live confirmations carry the disclaimer", () => {
    expect(isLiveAction("arm TREND for live trading")).toBe(true);
    expect(isLiveAction("switch live trading on")).toBe(true);
    expect(isLiveAction("disarm TREND")).toBe(false);
  });
});
