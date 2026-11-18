import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { MarketStatusDto } from "../ipc";
import { marketStatus, onMarketSession } from "../ipc";
import { MarketIndicator } from "./MarketIndicator";

vi.mock("../ipc", () => ({
  inTauri: () => true,
  marketStatus: vi.fn(),
  onMarketSession: vi.fn(),
}));

const open: MarketStatusDto = {
  session: "NORMAL",
  holiday: null,
  nextChangeUtcMs: Date.UTC(2026, 10, 18, 10, 0),
  calendarKnown: true,
};
const tick = () => new Promise<void>((r) => setTimeout(r, 10));

describe("MarketIndicator", () => {
  let dispose: (() => void) | undefined;
  let push: (s: MarketStatusDto) => void;
  let unlisten: ReturnType<typeof vi.fn>;
  let root: HTMLElement;

  beforeEach(() => {
    unlisten = vi.fn();
    vi.mocked(marketStatus).mockReset();
    vi.mocked(marketStatus).mockResolvedValue(open);
    vi.mocked(onMarketSession).mockImplementation(async (handler) => {
      push = handler;
      return unlisten as () => void;
    });
    root = document.createElement("div");
    document.body.append(root);
  });

  afterEach(() => {
    dispose?.();
    document.body.innerHTML = "";
    vi.restoreAllMocks();
  });

  it("reads the status once and then follows the session event, without polling", async () => {
    const interval = vi.spyOn(globalThis, "setInterval");
    dispose = render(() => <MarketIndicator />, root);
    await tick();
    expect(root.textContent).toContain("MARKET OPEN");
    push({ ...open, session: "CLOSED" });
    await tick();
    expect(root.textContent).toContain("MARKET CLOSED");
    expect(marketStatus).toHaveBeenCalledTimes(1);
    expect(interval).not.toHaveBeenCalled();
  });

  it("an event that arrives before the snapshot wins", async () => {
    vi.mocked(marketStatus).mockImplementation(async () => {
      await tick();
      return open;
    });
    dispose = render(() => <MarketIndicator />, root);
    await new Promise<void>((r) => setTimeout(r, 0));
    push({ ...open, session: "PRE_OPEN" });
    await tick();
    await tick();
    expect(root.textContent).toContain("PRE-OPEN");
  });

  it("stops listening when unmounted", async () => {
    dispose = render(() => <MarketIndicator />, root);
    await tick();
    dispose();
    dispose = undefined;
    expect(unlisten).toHaveBeenCalledTimes(1);
  });
});
