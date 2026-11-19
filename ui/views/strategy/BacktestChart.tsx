// The backtest's chart: the run's candles with an arrow at every fill,
// then the equity curve and the drawdown under it, one pane each. The
// curves are computed in Rust; this only draws them. The chart library is
// lazy-loaded, like the main chart's.

import type { IChartApi, UTCTimestamp } from "lightweight-charts";
import { type Component, createEffect, createResource, onCleanup, onMount, Show } from "solid-js";
import { describeError } from "../../data/errors";
import type { BacktestResultDto, HistoryDto } from "../../ipc";
import { getHistory, inTauri } from "../../ipc";
import { formatPrice } from "../../lib/format";
import { loadChartLib } from "../chart/lib";
import { axisLabel, candlePoints, crosshairLabel, isIntraday, parseInterval } from "../chart/model";
import { drawdownPoints, equityPoints, splitInstrument, tradeMarkers } from "./btChart";

const css = (name: string, fallback: string) =>
  getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;

export const BacktestChart: Component<{ result: BacktestResultDto }> = (props) => {
  const [history] = createResource(
    () => (inTauri() ? props.result : false),
    async (r): Promise<HistoryDto> => {
      const { exchange, symbol } = splitInstrument(r.symbol);
      // The run's own bars: normally a cache hit, the backtest just loaded them.
      return getHistory(exchange, symbol, r.interval.toLowerCase(), r.range.toLowerCase());
    },
  );

  const intraday = () => {
    const interval = parseInterval(props.result.interval);
    return interval !== undefined && isIntraday(interval);
  };

  let host: HTMLDivElement | undefined;
  let chart: IChartApi | undefined;
  let draw: ((r: BacktestResultDto, h: HistoryDto) => void) | undefined;
  let libError: string | undefined;

  onMount(async () => {
    if (!host) return;
    let lib: Awaited<ReturnType<typeof loadChartLib>>;
    try {
      lib = await loadChartLib();
    } catch (err) {
      libError = describeError(err);
      return;
    }
    if (!host) return;
    const up = css("--c-up", "#2bd47d");
    const down = css("--c-down", "#ff5252");
    const amber = css("--c-amber", "#ffb000");
    const line = css("--c-line", "#262626");
    const made = lib.createChart(host, {
      autoSize: true,
      layout: {
        background: { type: lib.ColorType.Solid, color: css("--c-bg-panel", "#070707") },
        textColor: css("--c-fg-dim", "#8c8c8c"),
        fontFamily: css("--font-mono", "monospace"),
        fontSize: 11,
        panes: { separatorColor: line, enableResize: true },
      },
      grid: { vertLines: { color: line }, horzLines: { color: line } },
      rightPriceScale: { borderColor: line },
      timeScale: {
        borderColor: line,
        timeVisible: intraday(),
        secondsVisible: false,
        // Bars are stored in UTC and shown in IST, like the main chart.
        tickMarkFormatter: (time: UTCTimestamp) => axisLabel(time, intraday(), false),
      },
      localization: {
        timeFormatter: (time: UTCTimestamp) => crosshairLabel(time, intraday()),
        priceFormatter: (p: number) => formatPrice(p),
      },
    });
    chart = made;
    const candles = made.addSeries(lib.CandlestickSeries, {
      upColor: up,
      downColor: down,
      wickUpColor: up,
      wickDownColor: down,
      borderVisible: false,
    });
    const markers = lib.createSeriesMarkers(candles, []);
    const equity = made.addSeries(
      lib.LineSeries,
      { color: amber, lineWidth: 1, priceLineVisible: false },
      1,
    );
    const drawdown = made.addSeries(
      lib.HistogramSeries,
      { color: `${down}99`, priceLineVisible: false, lastValueVisible: false },
      2,
    );
    made.panes()[1]?.setStretchFactor(0.35);
    made.panes()[2]?.setStretchFactor(0.25);
    draw = (r, h) => {
      candles.setData(candlePoints(h) as never);
      markers.setMarkers(
        tradeMarkers(r.trades, { buy: css("--c-info", "#5cb8ff"), win: up, loss: down }) as never,
      );
      equity.setData(equityPoints(r.curve) as never);
      drawdown.setData(drawdownPoints(r.curve) as never);
      made.timeScale().fitContent();
    };
    const h = history();
    if (h) draw(props.result, h);
  });

  createEffect(() => {
    const h = history();
    if (h && draw) draw(props.result, h);
  });

  onCleanup(() => chart?.remove());

  return (
    <div class="bt__chart-wrap">
      <Show when={history.error || libError}>
        <p class="view__error" role="alert">
          Chart unavailable: {history.error ? describeError(history.error) : libError}
        </p>
      </Show>
      <Show when={props.result.curve.length === 0}>
        <p class="strat__hint">No equity curve for this run. Run it again to draw one.</p>
      </Show>
      <div
        class="bt__chart"
        ref={host}
        role="img"
        aria-label={`Backtest chart of ${props.result.symbol}: candles with trade markers, equity and drawdown`}
      />
    </div>
  );
};
