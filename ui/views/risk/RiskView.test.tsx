import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cancelChallenge, confirmable } from "../../data/risk";
import type { RiskStatusDto } from "../../ipc";
import { riskStatus } from "../../ipc";
import { RiskView, usageRows, usedPct } from "./RiskView";

vi.mock("../../ipc", () => ({
  inTauri: () => true,
  riskStatus: vi.fn(),
  IpcFailure: class extends Error {},
}));

const tick = () => new Promise<void>((r) => setTimeout(r, 40));

const status = (over: Partial<RiskStatusDto> = {}): RiskStatusDto => ({
  kill: {
    engaged: false,
    reason: null,
    detail: "",
    engagedAt: null,
    resettableAt: null,
    resetPhrase: "RESET KILL SWITCH",
  },
  liveEnabled: false,
  liveRoute: false,
  limits: {
    maxOrderQuantity: 1000,
    maxOrderValue: 500000,
    maxSymbolExposure: 1000000,
    maxOpenPositions: 5,
    maxOrdersPerMinute: 10,
    maxOrdersPerDay: 100,
    maxDailyLoss: 50000,
    priceBandPct: 5,
    products: ["delivery"],
    tradingWindow: "09:15–15:30",
  },
  usage: {
    dayPnl: -45000,
    openPositions: 1,
    largestExposure: 2000,
    ordersToday: 3,
    ordersLastMinute: 0,
  },
  strategies: [
    {
      name: "TREND",
      running: true,
      mode: "ARMED",
      armedUntil: 1_795_168_800_000,
      paused: null,
      feed: "LIVE",
      limits: null,
    },
  ],
  ...over,
});

describe("usage", () => {
  it("counts a loss against the daily cap and skips per-order limits", () => {
    const rows = usageRows(status());
    const loss = rows.find((r) => r.limit === "Daily loss");
    expect(loss && usedPct(loss)).toBe(90);
    const qty = rows.find((r) => r.limit === "Order quantity");
    expect(qty && usedPct(qty)).toBeNull();
  });

  it("a profit uses none of the loss cap", () => {
    const rows = usageRows(status({ usage: { ...status().usage, dayPnl: 1200 } }));
    expect(rows[0]?.used).toBe(0);
  });
});

describe("RiskView", () => {
  let dispose: (() => void) | undefined;
  let root: HTMLElement;
  const text = () => root.textContent ?? "";

  beforeEach(() => {
    vi.mocked(riskStatus).mockResolvedValue(status());
    root = document.createElement("div");
    document.body.append(root);
    dispose = render(() => <RiskView />, root);
  });

  afterEach(() => {
    dispose?.();
    root.remove();
    cancelChallenge();
  });

  it("shows the switches, usage and each strategy's mode", async () => {
    await tick();
    expect(text()).toContain("KILL SWITCH OFF");
    expect(text()).toContain("PAPER ONLY");
    expect(text()).toContain("NO LIVE ROUTE");
    expect(text()).toContain("Daily loss");
    expect(text()).toContain("90%");
    expect(text()).toContain("ARMED");
    expect(text()).toContain("none (cannot arm)");
  });

  it("asks for the phrase in a focused box, and Esc cancels it", async () => {
    await confirmable("switch live trading on", async () => ({
      done: false,
      confirm: "ENABLE LIVE TRADING",
    }));
    await tick();
    const input = root.querySelector<HTMLInputElement>("#risk-confirm");
    expect(input).not.toBeNull();
    expect(text()).toContain("ENABLE LIVE TRADING");
    expect(document.activeElement).toBe(input);
    input?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await tick();
    expect(root.querySelector("#risk-confirm")).toBeNull();
  });

  it("an engaged kill switch says why", async () => {
    vi.mocked(riskStatus).mockResolvedValue(
      status({ kill: { ...status().kill, engaged: true, detail: "daily loss ₹50,000 reached" } }),
    );
    dispose?.();
    dispose = render(() => <RiskView />, root);
    await tick();
    expect(text()).toContain("KILL SWITCH ENGAGED");
    expect(text()).toContain("daily loss ₹50,000 reached");
  });
});
