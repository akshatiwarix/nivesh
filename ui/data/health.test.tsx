import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ProviderHealth } from "../components/ProviderHealth";
import type { ProviderStatusDto } from "../ipc";
import { onProviderStatus, providerStatus } from "../ipc";
import { createProviderHealth } from "./health";

vi.mock("../ipc", () => ({
  inTauri: () => true,
  providerStatus: vi.fn(),
  onProviderStatus: vi.fn(),
}));

const status = (provider: string, state: ProviderStatusDto["state"], extra = {}) =>
  ({
    provider,
    state,
    consecutiveFailures: state === "HEALTHY" ? 0 : 3,
    lastError: state === "HEALTHY" ? null : "blocked by the provider",
    lastOkMs: null,
    blocked: false,
    ...extra,
  }) satisfies ProviderStatusDto;

const tick = () => new Promise<void>((r) => setTimeout(r, 10));

describe("provider health in the status bar", () => {
  let dispose: (() => void) | undefined;
  let push: (s: ProviderStatusDto) => void;
  let unlisten: ReturnType<typeof vi.fn>;
  let root: HTMLElement;

  beforeEach(() => {
    unlisten = vi.fn();
    vi.mocked(providerStatus).mockResolvedValue([
      status("yahoo", "HEALTHY"),
      status("nse", "HEALTHY"),
    ]);
    vi.mocked(onProviderStatus).mockImplementation(async (handler) => {
      push = handler;
      return unlisten as () => void;
    });
    root = document.createElement("div");
    document.body.append(root);
    dispose = render(() => {
      const health = createProviderHealth();
      return <ProviderHealth health={health} />;
    }, root);
  });

  afterEach(() => {
    dispose?.();
    document.body.innerHTML = "";
  });

  const badges = () => [...root.querySelectorAll(".badge")].map((b) => b.textContent);

  it("shows each provider's state from the initial snapshot, sorted", async () => {
    await tick();
    expect(badges()).toEqual(["NSE OK", "YAHOO OK"]);
    expect(root.querySelector(".badge--up")).not.toBeNull();
  });

  it("turns a provider red when the backend reports it down, with the reason on hover", async () => {
    await tick();
    push(status("nse", "DOWN"));
    await tick();
    expect(badges()).toEqual(["NSE DOWN -> YAHOO", "YAHOO OK"]);
    const nse = root.querySelector(".badge--down");
    expect(nse?.getAttribute("title")).toContain("blocked by the provider");
    expect(nse?.getAttribute("title")).toContain("YAHOO is used instead");
    push(status("nse", "DEGRADED"));
    await tick();
    expect(badges()[0]).toBe("NSE SLOW");
    push(status("nse", "HEALTHY"));
    await tick();
    expect(badges()[0]).toBe("NSE OK");
  });

  it("says BLOCKED when the provider refuses us, and who covers for it", async () => {
    await tick();
    push(status("nse", "DOWN", { blocked: true }));
    await tick();
    expect(badges()).toEqual(["NSE BLOCKED -> YAHOO", "YAHOO OK"]);
    const title = root.querySelector(".badge--down")?.getAttribute("title") ?? "";
    expect(title).toContain("NSE is refusing requests from this network");
    expect(title).toContain("YAHOO is used instead");
  });

  it("names no stand-in when the fallback is down too, or for a source with none", async () => {
    await tick();
    push(status("nse", "DOWN", { blocked: true }));
    push(status("yahoo", "DOWN"));
    await tick();
    expect(badges()).toEqual(["NSE BLOCKED", "YAHOO DOWN"]);
    // Yahoo being down is not "covered" by NSE: NSE is the primary, not a fallback.
    push(status("nse", "HEALTHY"));
    await tick();
    expect(badges()).toEqual(["NSE OK", "YAHOO DOWN"]);
  });

  it("an event that beats the snapshot is not overwritten by it", async () => {
    vi.mocked(providerStatus).mockReset();
    vi.mocked(providerStatus).mockImplementation(async () => {
      await tick();
      return [status("nse", "HEALTHY")];
    });
    dispose?.();
    root.innerHTML = "";
    dispose = render(() => {
      const health = createProviderHealth();
      return <ProviderHealth health={health} />;
    }, root);
    await new Promise<void>((r) => setTimeout(r, 0));
    push(status("nse", "DOWN"));
    await tick();
    await tick();
    expect(badges()).toEqual(["NSE DOWN"]);
  });

  it("stops listening when the status bar goes away", async () => {
    await tick();
    dispose?.();
    dispose = undefined;
    expect(unlisten).toHaveBeenCalledTimes(1);
  });
});
