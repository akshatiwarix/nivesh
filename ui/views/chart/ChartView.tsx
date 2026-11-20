import type {
  IChartApi,
  ISeriesApi,
  LogicalRange as LibRange,
  MouseEventParams,
  SeriesType,
  UTCTimestamp,
} from "lightweight-charts";
import {
  type Component,
  createEffect,
  createMemo,
  createResource,
  createSignal,
  For,
  on,
  onCleanup,
  onMount,
  Show,
} from "solid-js";
import { DataPanel } from "../../components/DataPanel";
import { describeError } from "../../data/errors";
import type { BarDto, HistoryDto, ProvenanceDto } from "../../ipc";
import {
  chartIndicators,
  computeIndicators,
  getHistory,
  inTauri,
  previewIndicators,
  saveChartIndicators,
  subscribeFormingBar,
} from "../../ipc";
import {
  changeTone,
  formatCount,
  formatMovePct,
  formatPrice,
  formatVolumeShort,
} from "../../lib/format";
import { useViewCommands, useViewOps } from "../context";
import type { View } from "../types";
import "./ChartView.css";
import {
  histogramPoints,
  linePoints,
  paletteColor,
  removeSpecs,
  specText,
  splitByPlacement,
} from "./indicators";
import { type ChartLib, loadChartLib } from "./lib";
import {
  axisLabel,
  type Bar,
  barAt,
  type ChartInterval,
  type ChartRange,
  type Colors,
  candlePoints,
  crosshairLabel,
  INTERVAL_LABEL,
  INTERVALS,
  indexOfTime,
  initialRange,
  isIntraday,
  panRange,
  stepInterval,
  volumePoints,
  zoomRange,
} from "./model";

type ChartViewDef = Extract<View, { kind: "chart" }>;

const css = (name: string, fallback: string) =>
  getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;

/** Canvas colours come from the same tokens as the rest of the UI. */
function themeColors() {
  const up = css("--c-up", "#2bd47d");
  const down = css("--c-down", "#ff5252");
  return {
    bg: css("--c-bg-panel", "#070707"),
    fg: css("--c-fg-dim", "#8c8c8c"),
    line: css("--c-line", "#262626"),
    font: css("--font-mono", "monospace"),
    amber: css("--c-amber", "#ffb000"),
    colors: { up, down, upDim: `${up}66`, downDim: `${down}66` } satisfies Colors,
  };
}

interface ChartHandle {
  lib: ChartLib;
  chart: IChartApi;
  candles: ISeriesApi<"Candlestick">;
  volume: ISeriesApi<"Histogram">;
  colors: Colors;
  /** Line colours for indicators, in the order they are handed out. */
  palette: string[];
  /** Series drawn for indicators, removed and rebuilt when they change. */
  extra: ISeriesApi<SeriesType>[];
  /** One ref per indicator line, for moving the newest point as the forming
   * bar updates. Same order as `extra`. */
  indicatorLines: IndicatorLineRef[];
}

interface IndicatorLineRef {
  /** Canonical spec id, e.g. `SMA 50`. */
  id: string;
  series: ISeriesApi<SeriesType>;
  histogram: boolean;
  /** Pane the line lives in (0 = price pane). */
  pane: number;
}

/** Runs `run` on the next animation frame (a timer where there are none, as in tests). */
function schedule(run: () => void): number {
  return typeof requestAnimationFrame === "function"
    ? requestAnimationFrame(() => run())
    : (setTimeout(run, 16) as unknown as number);
}

function cancelFrame(id: number): void {
  if (typeof cancelAnimationFrame === "function") cancelAnimationFrame(id);
  else clearTimeout(id);
}

const ZOOM_STEP = 0.75;
const PAN_STEP = 0.2;

/**
 * A candlestick chart with a volume pane. Candles come from the backend as
 * columns; the forming bar is built in Rust and only drawn here. The library is
 * fetched on first use.
 */
export const ChartView: Component<{ view: ChartViewDef }> = (props) => {
  const [interval, setInterval] = createSignal<ChartInterval>(props.view.interval);
  const [range, setRange] = createSignal<ChartRange | undefined>(props.view.range);
  const [hover, setHover] = createSignal<Bar>();
  const [live, setLive] = createSignal<{ bar: Bar; provenance: ProvenanceDto }>();
  const [libError, setLibError] = createSignal<string>();

  const source = () => ({
    exchange: props.view.exchange,
    symbol: props.view.symbol,
    interval: interval(),
    range: range(),
  });
  const [history] = createResource(
    () => (inTauri() ? source() : false),
    async (s): Promise<HistoryDto> => getHistory(s.exchange, s.symbol, s.interval, s.range),
  );
  // Keep drawing the previous series while the next one loads.
  const data = () => (history.error ? undefined : history.latest);
  const failure = () => (history.error ? describeError(history.error) : undefined);

  let host: HTMLDivElement | undefined;
  let handle: ChartHandle | undefined;
  const [ready, setReady] = createSignal(false);

  onMount(async () => {
    if (!host) return;
    let lib: ChartLib;
    try {
      lib = await loadChartLib();
    } catch (err) {
      setLibError(describeError(err));
      return;
    }
    if (!host) return; // closed while the library was loading
    const theme = themeColors();
    const chart = lib.createChart(host, {
      autoSize: true,
      layout: {
        background: { type: lib.ColorType.Solid, color: theme.bg },
        textColor: theme.fg,
        fontFamily: theme.font,
        fontSize: 11,
        panes: { separatorColor: theme.line, enableResize: true },
      },
      grid: { vertLines: { color: theme.line }, horzLines: { color: theme.line } },
      rightPriceScale: { borderColor: theme.line },
      crosshair: { mode: lib.CrosshairMode.Normal },
      timeScale: {
        borderColor: theme.line,
        timeVisible: true,
        secondsVisible: false,
        tickMarkFormatter: (time: UTCTimestamp, _type: unknown, _locale: string) =>
          axisLabel(time, isIntraday(interval()), false),
      },
      localization: {
        timeFormatter: (time: UTCTimestamp) => crosshairLabel(time, isIntraday(interval())),
        priceFormatter: (p: number) => formatPrice(p),
      },
    });
    const candles = chart.addSeries(lib.CandlestickSeries, {
      upColor: theme.colors.up,
      downColor: theme.colors.down,
      wickUpColor: theme.colors.up,
      wickDownColor: theme.colors.down,
      borderVisible: false,
      priceLineColor: theme.amber,
    });
    const volume = chart.addSeries(
      lib.HistogramSeries,
      {
        // Its own formatter: the chart-wide one gives prices two decimals.
        priceFormat: { type: "custom", formatter: formatVolumeShort, minMove: 1 },
        priceLineVisible: false,
        lastValueVisible: false,
      },
      1,
    );
    chart.panes()[1]?.setStretchFactor(0.22);
    chart.subscribeCrosshairMove((p: MouseEventParams) => {
      if (p.time === undefined || !p.point) {
        setHover(undefined);
        return;
      }
      const h = data();
      if (!h) return;
      setHover(barAt(h, indexOfTime(h.t, (p.time as number) * 1000)));
    });
    handle = {
      lib,
      chart,
      candles,
      volume,
      colors: theme.colors,
      palette: [
        theme.amber,
        css("--c-info", "#5cb8ff"),
        css("--c-paper", "#c792ea"),
        css("--c-orange", "#ff7a1a"),
        css("--c-warn", "#ffd04d"),
      ],
      extra: [],
      indicatorLines: [],
    };
    setReady(true);
  });

  // Draw a freshly loaded series and reset the view to its newest bars.
  const [renderMs, setRenderMs] = createSignal<number>();
  createEffect(
    on([ready, data], ([isReady, h]) => {
      if (!isReady || !h || !handle) return;
      const began = performance.now();
      // Time from handing over the data to the first painted frame (dev builds show it).
      schedule(() => schedule(() => setRenderMs(performance.now() - began)));
      handle.candles.setData(candlePoints(h) as never);
      handle.volume.setData(volumePoints(h, handle.colors) as never);
      const r = initialRange(h.t.length);
      handle.chart.timeScale().setVisibleLogicalRange(r as LibRange);
      setLive(undefined);
      setHover(undefined);
    }),
  );

  // Indicators are saved per symbol, so they follow it across intervals.
  const [specs, setSpecs] = createSignal<string[]>([]);
  const [specsLoaded, setSpecsLoaded] = createSignal(false);
  onMount(() => {
    if (!inTauri()) return;
    chartIndicators(props.view.exchange, props.view.symbol)
      .then(setSpecs)
      .catch(() => undefined)
      .finally(() => setSpecsLoaded(true));
  });
  const persist = (next: string[]) => {
    setSpecs(next);
    void saveChartIndicators(props.view.exchange, props.view.symbol, next).catch(() => undefined);
  };

  const [indicators] = createResource(
    () => {
      const h = data();
      return h && specsLoaded() && specs().length > 0 ? { h, specs: specs() } : false;
    },
    ({ h, specs: list }) => computeIndicators(h.exchange, h.symbol, h.interval, h.range, list),
  );

  // Rebuild indicator series whenever the candles or the indicator set change.
  createEffect(
    on([ready, data, () => indicators.latest, specs], ([isReady, h, result, list]) => {
      if (!isReady || !handle || !h) return;
      const hd = handle;
      for (const s of hd.extra) hd.chart.removeSeries(s);
      hd.extra = [];
      hd.indicatorLines = [];
      if (!result || list.length === 0 || result.t.length !== h.t.length) return;
      const { lib, chart } = hd;
      const { overlays, panes } = splitByPlacement(result.indicators);
      let color = 0;
      for (const ind of overlays) {
        for (const line of ind.lines) {
          const dim = ind.kind === "BOLL" && line.name !== ind.id;
          const s = chart.addSeries(
            lib.LineSeries,
            {
              color: dim ? `${paletteColor(hd.palette, color)}99` : paletteColor(hd.palette, color),
              lineWidth: 1,
              priceLineVisible: false,
              lastValueVisible: !dim,
              crosshairMarkerVisible: false,
              title: dim ? "" : line.name,
            },
            0,
          );
          s.setData(linePoints(result.t, line) as never);
          hd.extra.push(s);
          hd.indicatorLines.push({ id: ind.id, series: s, histogram: false, pane: 0 });
        }
        color++;
      }
      // Volume is pane 1, so indicator panes start at 2.
      panes.forEach((ind, k) => {
        const paneIndex = 2 + k;
        ind.lines.forEach((line, n) => {
          const common = {
            priceLineVisible: false,
            lastValueVisible: n > 0 || ind.lines.length === 1,
          };
          const s =
            line.style === "histogram"
              ? chart.addSeries(
                  lib.HistogramSeries,
                  { ...common, lastValueVisible: false },
                  paneIndex,
                )
              : chart.addSeries(
                  lib.LineSeries,
                  {
                    ...common,
                    color: paletteColor(hd.palette, color + n),
                    lineWidth: 1,
                    title: line.name,
                    ...(ind.bounds
                      ? {
                          autoscaleInfoProvider: () => ({
                            priceRange: {
                              minValue: ind.bounds?.[0] ?? 0,
                              maxValue: ind.bounds?.[1] ?? 100,
                            },
                          }),
                        }
                      : {}),
                  },
                  paneIndex,
                );
          s.setData(
            (line.style === "histogram"
              ? histogramPoints(result.t, line, { up: hd.colors.upDim, down: hd.colors.downDim })
              : linePoints(result.t, line)) as never,
          );
          hd.indicatorLines.push({
            id: ind.id,
            series: s,
            histogram: line.style === "histogram",
            pane: paneIndex,
          });
          if (n === 0 || ind.lines.length === 1) {
            for (const g of ind.guides) {
              s.createPriceLine({
                price: g,
                color: themeColors().line,
                lineWidth: 1,
                lineStyle: 2,
                axisLabelVisible: false,
                title: "",
              });
            }
          }
          hd.extra.push(s);
        });
        color++;
        hd.chart.panes()[paneIndex]?.setStretchFactor(0.2);
      });
    }),
  );

  useViewCommands((name, args) => {
    if (name === "add") {
      const text = specText(args);
      if (text === "") return "ADD needs an indicator, e.g. ADD SMA 50";
      return computeIndicators(props.view.exchange, props.view.symbol, interval(), range(), [
        text,
      ]).then(
        (r) => {
          const id = r.indicators[0]?.id ?? text;
          if (specs().includes(id)) return `${id} is already on this chart`;
          persist([...specs(), id]);
          return undefined;
        },
        (err) => describeError(err),
      );
    }
    const result = removeSpecs(specs(), args);
    if ("error" in result) return result.error;
    persist(result.remaining);
    return undefined;
  });

  // The forming bar: built in Rust from the quote stream, only drawn here.
  createEffect(
    on([ready, data], ([isReady, h]) => {
      if (!isReady || !h) return;
      let stop: (() => void) | undefined;
      let closed = false;
      // Bars can arrive faster than frames; only the newest is drawn, once per
      // animation frame.
      let latest: BarDto | undefined;
      let frame: number | undefined;
      // Indicators: the series' newest point moves with the forming bar, so a
      // tick shows SMA/RSI/MACD on the live bar, not one bar behind. Recompute
      // at most twice a second — indicators don't move faster than that.
      let lastPreviewMs = 0;
      let previewBusy = false;
      const preview = (b: BarDto) => {
        if (closed || !handle || specs().length === 0 || previewBusy) return;
        const now = Date.now();
        if (now - lastPreviewMs < 600) return;
        lastPreviewMs = now;
        previewBusy = true;
        previewIndicators(h.exchange, h.symbol, h.interval, h.range, specs(), b)
          .then((p) => {
            previewBusy = false;
            const hd = handle;
            if (closed || !hd) return;
            const time = (p.t / 1000) as UTCTimestamp;
            for (const ind of p.indicators) {
              const refs = hd.indicatorLines.filter((r) => r.id === ind.id);
              refs.forEach((r, k) => {
                const v = ind.lines[k];
                if (v === null || v === undefined || !Number.isFinite(v)) {
                  // Whitespace keeps the line honest through warm-up.
                  r.series.update({ time } as never);
                } else if (r.histogram) {
                  r.series.update({
                    time,
                    value: v,
                    color: v >= 0 ? hd.colors.upDim : hd.colors.downDim,
                  } as never);
                } else {
                  r.series.update({ time, value: v } as never);
                }
              });
            }
          })
          .catch(() => {
            previewBusy = false;
          });
      };
      const draw = () => {
        frame = undefined;
        const b = latest;
        latest = undefined;
        if (closed || !handle || !b) return;
        const time = (b.t / 1000) as UTCTimestamp;
        handle.candles.update({ time, open: b.o, high: b.h, low: b.l, close: b.c });
        handle.volume.update({
          time,
          value: b.v,
          color: b.c >= b.o ? handle.colors.upDim : handle.colors.downDim,
        });
        setLive({ bar: b, provenance: b.provenance });
        preview(b);
      };
      subscribeFormingBar(h.exchange, h.symbol, h.interval, h.range, (b: BarDto) => {
        latest = b;
        frame ??= schedule(draw);
      })
        .then((unsubscribe) => {
          if (closed) unsubscribe();
          else stop = unsubscribe;
        })
        .catch(() => undefined);
      onCleanup(() => {
        closed = true;
        stop?.();
        if (frame !== undefined) cancelFrame(frame);
      });
    }),
  );

  onCleanup(() => {
    handle?.chart.remove();
    handle = undefined;
    host = undefined;
  });

  // Keyboard: the command bar keeps focus, so these arrive as view ops.
  const withRange = (f: (r: LibRange, total: number) => LibRange) => {
    const h = data();
    const scale = handle?.chart.timeScale();
    const r = scale?.getVisibleLogicalRange();
    if (!h || !scale || !r) return;
    scale.setVisibleLogicalRange(f(r, h.t.length));
  };
  useViewOps((op) => {
    switch (op) {
      case "zoomIn":
        withRange((r, n) => zoomRange(r, ZOOM_STEP, n) as LibRange);
        break;
      case "zoomOut":
        withRange((r, n) => zoomRange(r, 1 / ZOOM_STEP, n) as LibRange);
        break;
      case "panLeft":
        withRange((r) => panRange(r, -PAN_STEP) as LibRange);
        break;
      case "panRight":
        withRange((r) => panRange(r, PAN_STEP) as LibRange);
        break;
      case "home":
        withRange((r) => ({ from: 0, to: r.to - r.from }) as LibRange);
        break;
      case "end":
        withRange((r, n) => {
          const span = r.to - r.from;
          return { from: n + 4 - span, to: n + 4 } as LibRange;
        });
        break;
      case "tabPrev":
        setInterval((i) => stepInterval(i, -1));
        setRange(undefined);
        break;
      case "tabNext":
        setInterval((i) => stepInterval(i, 1));
        setRange(undefined);
        break;
    }
  });

  // What the readout shows: the bar under the crosshair, else the newest bar.
  const shown = createMemo<Bar | undefined>(() => {
    const h = data();
    if (!h) return undefined;
    return hover() ?? live()?.bar ?? barAt(h, h.t.length - 1);
  });
  const previousClose = createMemo(() => {
    const h = data();
    const b = shown();
    if (!h || !b) return undefined;
    const i = indexOfTime(h.t, b.t);
    return i > 0 ? (h.c[i - 1] as number) : b.o;
  });
  const change = () => {
    const b = shown();
    const p = previousClose();
    if (!b || p === undefined || p === 0) return undefined;
    return { abs: b.c - p, pct: ((b.c - p) / p) * 100 };
  };

  // The chart's provenance is its history's, with the as-of time moved along
  // by the forming bar (which comes from the same provider by construction).
  const provenance = createMemo<ProvenanceDto | null>(() => {
    const h = data();
    if (!h) return null;
    const l = live();
    return l && l.provenance.provider === h.provenance.provider
      ? { ...h.provenance, asOfMs: Math.max(h.provenance.asOfMs, l.provenance.asOfMs) }
      : h.provenance;
  });

  const emptyText = () => {
    if (!inTauri()) return "Charts are available in the Nivesh app.";
    if (libError()) return `Could not load the chart: ${libError()}`;
    if (failure()) return failure();
    return history.loading ? "Loading chart…" : undefined;
  };

  return (
    <div class="view chart" data-view="chart">
      <DataPanel
        provenance={provenance()}
        title={`${props.view.symbol} · ${props.view.exchange} · ${INTERVAL_LABEL[interval()]}`}
      >
        <div class="chart__bar" role="toolbar" aria-label="Interval">
          <For each={INTERVALS}>
            {(i) => (
              <button
                type="button"
                class="chart__interval"
                classList={{ "chart__interval--on": i === interval() }}
                aria-pressed={i === interval()}
                tabIndex={-1}
                onClick={() => {
                  setInterval(i);
                  setRange(undefined);
                }}
              >
                {INTERVAL_LABEL[i]}
              </button>
            )}
          </For>
          <Show when={data()}>
            {(h) => (
              <span class="chart__range" data-testid="chart-size">
                {h().range.toUpperCase()} · {formatCount(h().t.length)} bars
                <Show when={renderMs() !== undefined && import.meta.env.DEV}>
                  {` · drawn in ${renderMs()?.toFixed(0)} ms`}
                </Show>
              </span>
            )}
          </Show>
        </div>
        <Show when={specs().length > 0}>
          <div class="chart__legend" data-testid="indicators">
            <For each={specs()}>{(id) => <span class="chart__chip">{id}</span>}</For>
            <Show when={indicators.error}>
              <span class="chart__chip chart__chip--bad" role="alert">
                {describeError(indicators.error)}
              </span>
            </Show>
          </div>
        </Show>
        <Show when={shown()}>
          {(b) => (
            <div class="chart__readout" data-testid="readout">
              <span class="chart__cell">
                O <b>{formatPrice(b().o)}</b>
              </span>
              <span class="chart__cell">
                H <b>{formatPrice(b().h)}</b>
              </span>
              <span class="chart__cell">
                L <b>{formatPrice(b().l)}</b>
              </span>
              <span class="chart__cell">
                C <b>{formatPrice(b().c)}</b>
              </span>
              <Show when={change()}>
                {(c) => (
                  <span classList={{ [changeTone(c().abs) ?? ""]: true }}>
                    {formatMovePct(c().pct)}
                  </span>
                )}
              </Show>
              <span class="chart__cell">
                V <b>{formatCount(b().v)}</b>
              </span>
            </div>
          )}
        </Show>
        <Show when={emptyText()}>
          {(text) => (
            <p class="chart__msg" role={failure() ? "alert" : "status"}>
              {text()}
            </p>
          )}
        </Show>
        <div class="chart__host" ref={host} data-testid="chart-host" />
      </DataPanel>
    </div>
  );
};
