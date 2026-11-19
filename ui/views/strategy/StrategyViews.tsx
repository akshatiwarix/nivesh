// The strategy workspace: STRAT (the builder), BT (backtest results) and
// PAPER (the paper trading dashboard), all keyboard-driven.
//
// Every panel here is PAPER: simulated fills on simulated money, labelled
// as such by the shared badge. No view places a real order — Part 6, and
// only behind the risk engine.

import { type Component, createMemo, createResource, createSignal, Show } from "solid-js";
import { Badge } from "../../components/Badge";
import { DataPanel } from "../../components/DataPanel";
import { type Column, DataTable } from "../../components/DataTable";
import { describeError } from "../../data/errors";
import { createPoll } from "../../data/poll";
import type { BacktestResultDto, PaperFillDto, PaperStateDto, StrategyDto } from "../../ipc";
import { backtestResult, inTauri, paperStates, paperStop, strategies } from "../../ipc";
import { changeTone, formatPrice } from "../../lib/format";
import { formatIstDate, formatIstHm } from "../../lib/time";
import "./strategy.css";

const REFRESH_MS = 30_000;
/** The paper desk moves on every closed bar; poll it a little faster. */
const PAPER_REFRESH_MS = 5_000;

/** The PAPER badge every strategy surface carries. */
const PaperBadge: Component = () => <Badge tone="paper">PAPER</Badge>;

const STRATEGY_COLUMNS: readonly Column<StrategyDto>[] = [
  { id: "name", header: "Name", width: "22ch", value: (s) => s.name, tone: () => "accent" },
  {
    id: "template",
    header: "Template",
    width: "18ch",
    value: (s) => s.template,
    tone: () => "dim",
  },
  { id: "entry", header: "Entry", value: (s) => s.entry, tone: () => "dim" },
  {
    id: "running",
    header: "Paper",
    width: "10ch",
    value: (s) => (s.running ? "RUNNING" : s.hasState ? "STOPPED" : "—"),
    tone: (s) => (s.running ? "up" : undefined),
  },
];

/** `STRAT` / F10: the strategy list, with the builder underneath. */
export const StrategiesView: Component = () => {
  const list = createPoll(
    () => inTauri(),
    () => strategies(),
    { everyMs: () => REFRESH_MS },
  );
  const rows = createMemo(() => list.data() ?? []);
  const [selected, setSelected] = createSignal<number>();

  const emptyText = () => {
    if (!inTauri()) return "Strategies are available in the Nivesh app.";
    if (list.error() && !list.data()) return `${list.error()}. Retrying.`;
    if (rows().length === 0)
      return "No strategies yet. Pick a template: STRAT MA, STRAT RSI, STRAT BREAKOUT.";
    return undefined;
  };

  return (
    <div class="view strat" data-view="strat">
      <DataPanel provenance={null} title="STRATEGIES">
        <PaperBadge />
        <Show when={list.data() && list.error()}>
          <p class="view__error" role="alert">
            Refresh failed: {list.error()}. Showing the last data.
          </p>
        </Show>
        <DataTable
          label="Strategies"
          columns={STRATEGY_COLUMNS}
          rows={rows()}
          selectedIndex={selected()}
          onRowMouseDown={setSelected}
          emptyText={emptyText()}
        />
        <p class="strat__hint">
          STRAT &lt;TEMPLATE&gt; [QTY] builds from a template: STRAT MA 20 50 1, STRAT RSI 14, STRAT
          BREAKOUT 20 · PAPER START/STOP runs or stops it
        </p>
      </DataPanel>
    </div>
  );
};

const TRADE_COLUMNS: readonly Column<BacktestResultDto["trades"][number]>[] = [
  {
    id: "entry",
    header: "Entry ₹",
    width: "12ch",
    align: "right",
    value: (t) => formatPrice(t.entryPrice),
  },
  {
    id: "exit",
    header: "Exit ₹",
    width: "12ch",
    align: "right",
    value: (t) => formatPrice(t.exitPrice),
  },
  { id: "qty", header: "Qty", width: "8ch", align: "right", value: (t) => String(t.quantity) },
  {
    id: "costs",
    header: "Costs ₹",
    width: "10ch",
    align: "right",
    value: (t) => formatPrice(t.charges),
    tone: () => "dim",
  },
  {
    id: "pnl",
    header: "Net P&L ₹",
    width: "12ch",
    align: "right",
    value: (t) => formatPrice(t.pnl),
    tone: (t) => (t.pnl > 0 ? "up" : t.pnl < 0 ? "down" : undefined),
  },
];

/** `BT <NAME>`: the latest backtest result of a strategy. */
export const BacktestView: Component<{ name?: string }> = (props) => {
  const [result] = createResource(
    () => inTauri() && props.name !== undefined,
    () => backtestResult(props.name ?? ""),
  );

  const emptyText = () => {
    if (!inTauri()) return "Backtests are available in the Nivesh app.";
    if (result.state === "pending") return "Running…";
    if (result.error) return `${describeError(result.error)}. Run it again: BT ${props.name}`;
    if (props.name === undefined) return "Which strategy? BT <NAME>.";
    return undefined;
  };

  const trades = createMemo(() => result()?.trades ?? []);

  return (
    <div class="view bt" data-view="bt">
      <DataPanel
        provenance={null}
        title={`BACKTEST · ${props.name ?? "—"}${
          result() ? ` · ${result()?.symbol} ${result()?.interval} ${result()?.range}` : ""
        }`}
      >
        <PaperBadge />
        <Show when={result()}>
          {(r) => (
            <p class="strat__funds">
              NET P&L {formatPrice(r().totalPnl)} · COSTS {formatPrice(r().totalCharges)} · MAX DD{" "}
              {formatPrice(r().maxDrawdown)} · TRADES {r().trades.length} · BARS {r().barsInMarket}/
              {r().totalBars}
            </p>
          )}
        </Show>
        <Show when={result.error || (result.state === "ready" && !result())}>
          <p class="view__error" role="alert">
            {emptyText()}
          </p>
        </Show>
        <DataTable
          label="Backtest trades"
          columns={TRADE_COLUMNS}
          rows={trades()}
          emptyText={emptyText()}
        />
        <p class="strat__hint">
          Simulated fills on simulated money, at the next bar's open. Net of Zerodha-style equity
          delivery charges (STT, exchange, SEBI, stamp, GST, DP) and 5 bps of slippage per fill.
        </p>
      </DataPanel>
    </div>
  );
};

type PaperRow = PaperStateDto;
type FillRow = PaperFillDto & { name: string };

const PAPER_COLUMNS: readonly Column<PaperRow>[] = [
  { id: "name", header: "Name", width: "18ch", value: (p) => p.name, tone: () => "accent" },
  {
    id: "stream",
    header: "Stream",
    width: "18ch",
    value: (p) => `${p.symbol} · ${p.interval}`,
  },
  {
    id: "status",
    header: "Status",
    width: "9ch",
    value: (p) => (p.running ? "RUNNING" : "STOPPED"),
    tone: (p) => (p.running ? "up" : "dim"),
  },
  {
    id: "feed",
    header: "Bars",
    width: "16ch",
    // DELAYED bars make the paper fills approximate: say so on the row.
    value: (p) => (p.approximate ? `${p.feed} · approx` : p.feed),
    tone: (p) => (p.approximate ? "dim" : undefined),
  },
  {
    id: "pos",
    header: "Position",
    width: "16ch",
    align: "right",
    value: (p) => (p.quantity === null ? "flat" : `${p.quantity} @ ${formatPrice(p.entryPrice)}`),
  },
  {
    id: "day",
    header: "Day P&L ₹",
    width: "12ch",
    align: "right",
    value: (p) => formatPrice(p.dayPnl),
    tone: (p) => changeTone(p.dayPnl),
  },
  {
    id: "total",
    header: "Net P&L ₹",
    width: "12ch",
    align: "right",
    value: (p) => formatPrice(p.realisedPnl),
    tone: (p) => changeTone(p.realisedPnl),
  },
  {
    id: "costs",
    header: "Costs ₹",
    width: "10ch",
    align: "right",
    value: (p) => formatPrice(p.charges),
    tone: () => "dim",
  },
];

const FILL_COLUMNS: readonly Column<FillRow>[] = [
  {
    id: "t",
    header: "Bar (IST)",
    width: "18ch",
    value: (f) => `${formatIstDate(f.t)} ${formatIstHm(f.t)}`,
  },
  { id: "name", header: "Strategy", width: "18ch", value: (f) => f.name, tone: () => "accent" },
  {
    id: "side",
    header: "Side",
    width: "6ch",
    value: (f) => f.side,
    tone: (f) => (f.side === "BUY" ? "up" : "down"),
  },
  { id: "qty", header: "Qty", width: "8ch", align: "right", value: (f) => String(f.quantity) },
  {
    id: "price",
    header: "Price ₹",
    width: "12ch",
    align: "right",
    value: (f) => formatPrice(f.price),
  },
  {
    id: "costs",
    header: "Costs ₹",
    width: "10ch",
    align: "right",
    value: (f) => formatPrice(f.charges),
    tone: () => "dim",
  },
  {
    id: "pnl",
    header: "P&L ₹",
    width: "12ch",
    align: "right",
    value: (f) => (f.pnl === null ? "" : formatPrice(f.pnl)),
    tone: (f) => changeTone(f.pnl),
  },
];

/** Most recent fills shown across strategies. */
const MAX_FILL_ROWS = 50;

/** `PAPER`: every paper strategy with its position, P&L and recent fills. */
export const PaperDashboardView: Component = () => {
  const list = createPoll(
    () => inTauri(),
    () => paperStates(),
    { everyMs: () => PAPER_REFRESH_MS },
  );
  const rows = createMemo(() => list.data() ?? []);
  const fills = createMemo(() =>
    rows()
      .flatMap((p) => p.fills.map((f) => ({ ...f, name: p.name })))
      .sort((a, b) => b.t - a.t)
      .slice(0, MAX_FILL_ROWS),
  );

  const emptyText = () => {
    if (!inTauri()) return "Paper trading is available in the Nivesh app.";
    if (list.error() && !list.data()) return `${list.error()}. Retrying.`;
    if (rows().length === 0)
      return "No paper strategy yet. PAPER START <NAME> [SYMBOL] [INTERVAL].";
    return undefined;
  };

  return (
    <div class="view paper" data-view="paper">
      <DataPanel provenance={null} title="PAPER TRADING">
        <PaperBadge />
        <Show when={list.data() && list.error()}>
          <p class="view__error" role="alert">
            Refresh failed: {list.error()}. Showing the last data.
          </p>
        </Show>
        <DataTable
          label="Paper strategies"
          columns={PAPER_COLUMNS}
          rows={rows()}
          emptyText={emptyText()}
        />
        <Show when={fills().length > 0}>
          <DataTable label="Paper fills" columns={FILL_COLUMNS} rows={fills()} />
        </Show>
        <p class="strat__hint">
          PAPER START &lt;NAME&gt; [SYMBOL] [INTERVAL] · PAPER STOP &lt;NAME&gt; · decisions on
          closed bars, fills at the next bar's open · every fill here is simulated; nothing reaches
          a real broker. On DELAYED bars the fills are approximate.
        </p>
      </DataPanel>
    </div>
  );
};

/** Stops a running paper strategy (the PAPER STOP path). */
export async function stopPaper(name: string): Promise<void> {
  await paperStop(name);
}
