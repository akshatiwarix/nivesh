// The strategy workspace: STRAT (the builder), BT (backtest results) and
// PAPER (the paper trading dashboard), all keyboard-driven.
//
// Every panel here is PAPER: simulated fills on simulated money, labelled
// as such by the shared badge. No view places a real order — Part 6, and
// only behind the risk engine.

import { type Component, createMemo, createResource, createSignal, For, Show } from "solid-js";
import { Badge } from "../../components/Badge";
import { DataPanel } from "../../components/DataPanel";
import { type Column, DataTable } from "../../components/DataTable";
import { describeError } from "../../data/errors";
import { createPoll } from "../../data/poll";
import type { BacktestResultDto, StrategyDto } from "../../ipc";
import { backtestResult, inTauri, paperStop, strategies } from "../../ipc";
import { formatPrice } from "../../lib/format";
import "./strategy.css";

const REFRESH_MS = 30_000;

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
    id: "pnl",
    header: "P&L ₹",
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
      <DataPanel provenance={null} title={`BACKTEST · ${props.name ?? "—"}`}>
        <PaperBadge />
        <Show when={result()}>
          {(r) => (
            <p class="strat__funds">
              P&L {formatPrice(r().totalPnl)} · MAX DD {formatPrice(r().maxDrawdown)} · TRADES{" "}
              {r().trades.length} · BARS {r().barsInMarket}/{r().totalBars}
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
          Simulated fills on simulated money. Fees and slippage are not modelled yet.
        </p>
      </DataPanel>
    </div>
  );
};

/** `PAPER`: the running paper strategy's state. */
export const PaperDashboardView: Component = () => {
  const list = createPoll(
    () => inTauri(),
    () => strategies(),
    { everyMs: () => REFRESH_MS },
  );
  const running = createMemo(() => (list.data() ?? []).filter((s) => s.running));

  const emptyText = () => {
    if (!inTauri()) return "Paper trading is available in the Nivesh app.";
    if (list.error() && !list.data()) return `${list.error()}. Retrying.`;
    if (running().length === 0) return "No paper strategy is running. PAPER START <NAME>.";
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
        <Show when={running().length > 0} fallback={<p class="strat__hint">{emptyText()}</p>}>
          <ul class="strat__list">
            <For each={running()}>
              {(s) => (
                <li>
                  <b>{s.name}</b> — {s.entry}
                  <Show when={s.paperPnl !== null}> · P&L {formatPrice(s.paperPnl ?? 0)}</Show>
                </li>
              )}
            </For>
          </ul>
        </Show>
        <p class="strat__hint">
          PAPER START &lt;NAME&gt; · PAPER STOP &lt;NAME&gt; · every fill here is simulated; nothing
          reaches a real broker.
        </p>
      </DataPanel>
    </div>
  );
};

/** Stops a running paper strategy (the PAPER STOP path). */
export async function stopPaper(name: string): Promise<void> {
  await paperStop(name);
}
