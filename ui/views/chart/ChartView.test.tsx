import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { pinClock, unpinClock } from "../../data/testClock";
import type { BarDto, HistoryDto } from "../../ipc";
import { getHistory, subscribeFormingBar } from "../../ipc";
import { hasViewOps, PanelContext, sendViewOp } from "../context";
import type { View } from "../types";
import { ChartView } from "./ChartView";

vi.mock("../../ipc", () => ({
  inTauri: () => true,
  getHistory: vi.fn(),
  subscribeFormingBar: vi.fn(),
  toIpcError: (e: unknown) => ({ code: "INTERNAL", message: String(e), retryable: false }),
  IpcFailure: class extends Error {},
}));

// The chart library draws on canvas, which the test DOM doesn't have. A fake
// records what the view asks of it.
const calls = vi.hoisted(() => ({
  setData: [] as { name: string; data: unknown[] }[],
  updates: [] as unknown[],
  ranges: [] as unknown[],
  created: 0,
  removed: 0,
}));
let visible = { from: 100, to: 200 };

vi.mock("./lib", () => {
  const series = (name: string) => ({
    setData: (data: unknown[]) => calls.setData.push({ name, data }),
    update: (p: unknown) => calls.updates.push({ name, p }),
  });
  return {
    loadChartLib: async () => ({
      ColorType: { Solid: "solid" },
      CrosshairMode: { Normal: 0 },
      CandlestickSeries: "candles",
      HistogramSeries: "histogram",
      createChart: () => {
        calls.created++;
        return {
          addSeries: (kind: string) => series(kind),
          panes: () => [{ setStretchFactor() {} }, { setStretchFactor() {} }],
          subscribeCrosshairMove() {},
          timeScale: () => ({
            getVisibleLogicalRange: () => visible,
            setVisibleLogicalRange: (r: unknown) => {
              calls.ranges.push(r);
              visible = r as typeof visible;
            },
          }),
          remove: () => {
            calls.removed++;
          },
        };
      },
    }),
  };
});

const getHistoryMock = vi.mocked(getHistory);
const subscribeMock = vi.mocked(subscribeFormingBar);
const tick = () => new Promise<void>((r) => setTimeout(r, 40));

const AS_OF = Date.UTC(2026, 10, 18, 5, 44, 0);
function history(over: Partial<HistoryDto> = {}): HistoryDto {
  const n = 200;
  const t = Array.from({ length: n }, (_, i) => Date.UTC(2026, 0, 1) + i * 86_400_000);
  return {
    exchange: "NSE",
    symbol: "RELIANCE",
    interval: "1d",
    range: "5y",
    t,
    o: t.map(() => 100),
    h: t.map(() => 110),
    l: t.map(() => 95),
    c: t.map((_, i) => 100 + i / 10),
    v: t.map(() => 5000),
    provenance: { provider: "yahoo", freshness: "DELAYED", asOfMs: AS_OF, fetchedAtMs: AS_OF },
    ...over,
  };
}

describe("ChartView", () => {
  let dispose: (() => void) | undefined;
  let root: HTMLElement;
  let push: (b: BarDto) => void;
  let stop: () => void;

  function mount(view: Extract<View, { kind: "chart" }>) {
    root = document.createElement("div");
    document.body.append(root);
    dispose = render(() => <ChartView view={view} />, root);
  }
  const view = (over: Partial<Extract<View, { kind: "chart" }>> = {}) =>
    ({ kind: "chart", exchange: "NSE", symbol: "RELIANCE", interval: "1d", ...over }) as Extract<
      View,
      { kind: "chart" }
    >;
  const text = () => root.textContent ?? "";

  beforeEach(() => {
    pinClock();
    stop = vi.fn();
    getHistoryMock.mockReset();
    subscribeMock.mockReset();
    subscribeMock.mockImplementation(async (_e, _s, _i, _r, onBar) => {
      push = onBar;
      return stop;
    });
    calls.setData.length = 0;
    calls.updates.length = 0;
    calls.ranges.length = 0;
    calls.created = 0;
    calls.removed = 0;
    visible = { from: 100, to: 200 };
  });

  afterEach(() => {
    unpinClock();
    dispose?.();
    document.body.innerHTML = "";
  });

  it("loads history for the view's interval and labels it DELAYED with source and time", async () => {
    getHistoryMock.mockResolvedValue(history());
    mount(view({ interval: "5m", range: "1mo" }));
    await tick();
    expect(getHistoryMock).toHaveBeenCalledWith("NSE", "RELIANCE", "5m", "1mo");
    expect(text()).toContain("DELAYED");
    expect(text()).toContain("YAHOO");
    expect(text()).toContain("IST");
  });

  it("draws candles and a volume pane, and opens on the newest bars", async () => {
    getHistoryMock.mockResolvedValue(history());
    mount(view());
    await tick();
    expect(calls.created).toBe(1);
    const names = calls.setData.map((s) => s.name).sort();
    expect(names).toEqual(["candles", "histogram"]);
    expect(calls.setData.find((s) => s.name === "candles")?.data).toHaveLength(200);
    expect(calls.ranges.at(-1)).toEqual({ from: 50, to: 204 });
  });

  it("shows the newest bar's OHLCV in the readout", async () => {
    getHistoryMock.mockResolvedValue(history());
    mount(view());
    await tick();
    const readout = root.querySelector('[data-testid="readout"]')?.textContent ?? "";
    expect(readout).toContain("119.90");
    expect(readout).toContain("5,000");
  });

  it("explains an unsupported range instead of showing an empty chart", async () => {
    getHistoryMock.mockRejectedValue(new Error("Yahoo: 1-minute bars cover only the last 7 days"));
    mount(view({ interval: "1m", range: "3mo" }));
    await tick();
    expect(root.querySelector('[role="alert"]')?.textContent).toContain("only the last 7 days");
    expect(calls.setData).toHaveLength(0);
  });

  it("draws the forming bar as it arrives and stops the stream on close", async () => {
    getHistoryMock.mockResolvedValue(history());
    mount(view());
    await tick();
    expect(subscribeMock).toHaveBeenCalledWith("NSE", "RELIANCE", "1d", "5y", expect.any(Function));
    const t = Date.UTC(2026, 6, 20);
    push({
      t,
      o: 120,
      h: 125,
      l: 119,
      c: 124,
      v: 0,
      provenance: {
        provider: "yahoo",
        freshness: "DELAYED",
        asOfMs: AS_OF + 5000,
        fetchedAtMs: AS_OF,
      },
    });
    expect(calls.updates).toContainEqual({
      name: "candles",
      p: { time: t / 1000, open: 120, high: 125, low: 119, close: 124 },
    });
    dispose?.();
    dispose = undefined;
    await tick();
    expect(stop).toHaveBeenCalled();
    expect(calls.removed).toBe(1);
  });

  it("view keys zoom, pan and step the interval", async () => {
    getHistoryMock.mockResolvedValue(history());
    root = document.createElement("div");
    document.body.append(root);
    dispose = render(
      () => (
        <PanelContext.Provider
          value={{ panel: 0, open: () => {}, openBeside: () => {}, focused: () => true }}
        >
          <ChartView view={view({ interval: "5m" })} />
        </PanelContext.Provider>
      ),
      root,
    );
    await tick();
    expect(hasViewOps(0)).toBe(true);

    visible = { from: 100, to: 200 };
    sendViewOp(0, "zoomIn");
    expect(calls.ranges.at(-1)).toEqual({ from: 125, to: 200 });
    sendViewOp(0, "zoomOut");
    sendViewOp(0, "panLeft");
    expect((calls.ranges.at(-1) as { to: number }).to).toBeLessThan(200);
    sendViewOp(0, "home");
    expect((calls.ranges.at(-1) as { from: number }).from).toBe(0);
    sendViewOp(0, "end");
    expect((calls.ranges.at(-1) as { to: number }).to).toBe(204);

    sendViewOp(0, "tabNext");
    await tick();
    expect(getHistoryMock).toHaveBeenLastCalledWith("NSE", "RELIANCE", "15m", undefined);
    sendViewOp(0, "tabPrev");
    sendViewOp(0, "tabPrev");
    await tick();
    expect(getHistoryMock).toHaveBeenLastCalledWith("NSE", "RELIANCE", "1m", undefined);
  });

  it("the toolbar switches interval with the mouse too", async () => {
    getHistoryMock.mockResolvedValue(history());
    mount(view({ interval: "5m" }));
    await tick();
    const buttons = [...root.querySelectorAll<HTMLButtonElement>(".chart__interval")];
    buttons.find((b) => b.textContent === "1H")?.click();
    await tick();
    expect(getHistoryMock).toHaveBeenLastCalledWith("NSE", "RELIANCE", "1h", undefined);
    expect(root.querySelector(".chart__interval--on")?.textContent).toBe("1H");
  });
});
