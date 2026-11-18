import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { pinClock, unpinClock } from "../../data/testClock";
import type { BarDto, HistoryDto } from "../../ipc";
import {
  chartIndicators,
  computeIndicators,
  getHistory,
  saveChartIndicators,
  subscribeFormingBar,
} from "../../ipc";
import { hasViewOps, PanelContext, sendViewCommand, sendViewOp } from "../context";
import type { View } from "../types";
import { ChartView } from "./ChartView";

vi.mock("../../ipc", () => ({
  inTauri: () => true,
  getHistory: vi.fn(),
  subscribeFormingBar: vi.fn(),
  chartIndicators: vi.fn(),
  computeIndicators: vi.fn(),
  saveChartIndicators: vi.fn(),
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
  added: [] as { kind: string; pane: number | undefined; data?: unknown[] }[],
  removedSeries: 0,
}));
let visible = { from: 100, to: 200 };

vi.mock("./lib", () => {
  const series = (name: string) => ({
    setData: (data: unknown[]) => calls.setData.push({ name, data }),
    update: (p: unknown) => calls.updates.push({ name, p }),
    createPriceLine: () => {},
  });
  return {
    loadChartLib: async () => ({
      ColorType: { Solid: "solid" },
      CrosshairMode: { Normal: 0 },
      CandlestickSeries: "candles",
      HistogramSeries: "histogram",
      LineSeries: "line",
      createChart: () => {
        calls.created++;
        return {
          addSeries: (kind: string, _opts: unknown, pane?: number) => {
            calls.added.push({ kind, pane });
            return series(kind === "line" ? `line@${pane}` : kind);
          },
          removeSeries: () => {
            calls.removedSeries++;
          },
          panes: () => [
            { setStretchFactor() {} },
            { setStretchFactor() {} },
            { setStretchFactor() {} },
          ],
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
    calls.added.length = 0;
    calls.removedSeries = 0;
    vi.mocked(chartIndicators).mockReset().mockResolvedValue([]);
    vi.mocked(computeIndicators).mockReset();
    vi.mocked(saveChartIndicators).mockReset().mockResolvedValue(undefined);
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
    // Drawn on the next animation frame, not synchronously.
    expect(calls.updates).toHaveLength(0);
    await tick();
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

  it("coalesces bars that arrive within one frame into a single draw of the newest", async () => {
    getHistoryMock.mockResolvedValue(history());
    mount(view());
    await tick();
    const provenance = {
      provider: "yahoo",
      freshness: "DELAYED" as const,
      asOfMs: AS_OF,
      fetchedAtMs: AS_OF,
    };
    const t = Date.UTC(2026, 6, 20);
    for (const c of [121, 122, 123]) push({ t, o: 120, h: c, l: 119, c, v: 0, provenance });
    await tick();
    const candleUpdates = calls.updates.filter((u) => (u as { name: string }).name === "candles");
    expect(candleUpdates).toHaveLength(1);
    expect((candleUpdates[0] as { p: { close: number } }).p.close).toBe(123);
  });

  it("shows how many bars were loaded", async () => {
    getHistoryMock.mockResolvedValue(history());
    mount(view());
    await tick();
    expect(root.querySelector('[data-testid="chart-size"]')?.textContent).toContain("200 bars");
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

  describe("indicators", () => {
    const indicatorsFor = (h: HistoryDto, ids: string[]) => ({
      t: h.t,
      indicators: ids.map((id) => {
        const rsi = id.startsWith("RSI");
        return {
          id,
          kind: id.split(" ")[0] ?? "",
          placement: rsi ? "pane" : "overlay",
          lines: [
            {
              name: id,
              style: "line",
              values: h.t.map((_, i) => (i < 5 ? null : 100 + i)),
            },
          ],
          bounds: rsi ? ([0, 100] as [number, number]) : null,
          guides: rsi ? [30, 70] : [],
        };
      }),
    });

    function mountWithPanel() {
      root = document.createElement("div");
      document.body.append(root);
      dispose = render(
        () => (
          <PanelContext.Provider
            value={{ panel: 0, open: () => {}, openBeside: () => {}, focused: () => true }}
          >
            <ChartView view={view()} />
          </PanelContext.Provider>
        ),
        root,
      );
    }

    it("restores a symbol's saved indicators: overlays on price, RSI in its own pane", async () => {
      const h = history();
      getHistoryMock.mockResolvedValue(h);
      vi.mocked(chartIndicators).mockResolvedValue(["SMA 50", "RSI 14"]);
      vi.mocked(computeIndicators).mockImplementation(async (_e, _s, _i, _r, specs) =>
        indicatorsFor(h, [...specs]),
      );
      mount(view());
      await tick();
      expect(chartIndicators).toHaveBeenCalledWith("NSE", "RELIANCE");
      expect(computeIndicators).toHaveBeenCalledWith("NSE", "RELIANCE", "1d", "5y", [
        "SMA 50",
        "RSI 14",
      ]);
      const lines = calls.added.filter((a) => a.kind === "line");
      // The SMA sits on the price pane (0); RSI in the first pane after volume (2).
      expect(lines.map((l) => l.pane)).toEqual([0, 2]);
      expect(root.querySelector('[data-testid="indicators"]')?.textContent).toContain("SMA 50");
      expect(root.querySelector('[data-testid="indicators"]')?.textContent).toContain("RSI 14");
    });

    it("ADD validates through Rust, saves the canonical form, and RM removes it", async () => {
      const h = history();
      getHistoryMock.mockResolvedValue(h);
      vi.mocked(computeIndicators).mockImplementation(async (_e, _s, _i, _r, specs) =>
        indicatorsFor(
          h,
          specs.map((s) => s.replace(/^RSI$/, "RSI 14")),
        ),
      );
      mountWithPanel();
      await tick();

      expect(await sendViewCommand(0, "add", ["rsi"])).toBeUndefined();
      await tick();
      expect(saveChartIndicators).toHaveBeenLastCalledWith("NSE", "RELIANCE", ["RSI 14"]);
      expect(calls.added.some((a) => a.kind === "line" && a.pane === 2)).toBe(true);
      // Adding it twice is refused with a message.
      expect(await sendViewCommand(0, "add", ["rsi"])).toMatch(/already on this chart/);

      expect(await sendViewCommand(0, "rm", ["RSI"])).toBeUndefined();
      await tick();
      expect(saveChartIndicators).toHaveBeenLastCalledWith("NSE", "RELIANCE", []);
      expect(calls.removedSeries).toBeGreaterThan(0);
      expect(await sendViewCommand(0, "rm", ["RSI"])).toMatch(/not on this chart/);
    });

    it("passes the backend's reason when an indicator is invalid", async () => {
      getHistoryMock.mockResolvedValue(history());
      vi.mocked(computeIndicators).mockRejectedValue(new Error("unknown indicator `FOO`"));
      mountWithPanel();
      await tick();
      expect(await sendViewCommand(0, "add", ["foo"])).toMatch(/unknown indicator/);
      expect(saveChartIndicators).not.toHaveBeenCalled();
    });
  });
});
