import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setWindowVisible } from "../ipc";
import { createVisibilityReporter } from "./visibility";

vi.mock("../ipc", () => ({
  inTauri: () => true,
  setWindowVisible: vi.fn().mockResolvedValue(undefined),
}));

const setHidden = (hidden: boolean) => {
  Object.defineProperty(document, "hidden", { configurable: true, get: () => hidden });
  document.dispatchEvent(new Event("visibilitychange"));
};

describe("window visibility reporting", () => {
  let dispose: (() => void) | undefined;
  afterEach(() => {
    dispose?.();
    vi.mocked(setWindowVisible).mockClear();
    setHidden(false);
  });

  const mount = () => {
    dispose = render(() => {
      createVisibilityReporter();
      return <div />;
    }, document.body);
  };

  it("tells the backend when the window is hidden and shown", () => {
    mount();
    setHidden(true);
    expect(setWindowVisible).toHaveBeenLastCalledWith(false);
    setHidden(false);
    expect(setWindowVisible).toHaveBeenLastCalledWith(true);
  });

  it("stops reporting once unmounted", () => {
    mount();
    dispose?.();
    dispose = undefined;
    vi.mocked(setWindowVisible).mockClear();
    setHidden(true);
    expect(setWindowVisible).not.toHaveBeenCalled();
  });

  it("a failing call is ignored", async () => {
    vi.mocked(setWindowVisible).mockRejectedValueOnce(new Error("gone"));
    mount();
    expect(() => setHidden(true)).not.toThrow();
    await new Promise<void>((r) => setTimeout(r, 5));
  });
});
