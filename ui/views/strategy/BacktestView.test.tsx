import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setBtRun } from "../../data/backtest";
import { backtestResult } from "../../ipc";
import { BacktestView } from "./StrategyViews";

vi.mock("../../ipc", async (actual) => ({
  ...(await actual<typeof import("../../ipc")>()),
  inTauri: () => true,
  backtestResult: vi.fn(),
}));
// The chart needs a real canvas; it has its own tests.
vi.mock("./BacktestChart", () => ({ BacktestChart: () => null }));

const tick = () => new Promise<void>((r) => setTimeout(r, 0));

const saved = (pnl: number) => ({
  symbol: "NSE:TCS",
  interval: "1D",
  range: "1Y",
  totalPnl: pnl,
  totalCharges: 0,
  maxDrawdown: 0,
  trades: [],
  barsInMarket: 0,
  totalBars: 0,
  walkForward: null,
  curve: [],
});

describe("BacktestView", () => {
  let dispose: (() => void) | undefined;
  const root = document.createElement("div");
  afterEach(() => dispose?.());

  it("shows Running… during a run, then the new result, not the old one", async () => {
    vi.mocked(backtestResult).mockResolvedValue(saved(111) as never);
    setBtRun("A", { state: "running" });
    dispose = render(() => <BacktestView name="A" />, root);
    await tick();
    expect(root.textContent).toContain("Running…");
    expect(backtestResult).not.toHaveBeenCalled();
    vi.mocked(backtestResult).mockResolvedValue(saved(222) as never);
    setBtRun("A", { state: "done", at: 1 });
    await tick();
    expect(root.textContent).toContain("222");
  });

  it("shows why a run failed instead of Running…", async () => {
    setBtRun("B", { state: "failed", error: 'no strategy named "B"' });
    dispose = render(() => <BacktestView name="B" />, root);
    await tick();
    expect(root.textContent).toContain('no strategy named "B"');
    expect(root.textContent).not.toContain("Running…");
  });
});
