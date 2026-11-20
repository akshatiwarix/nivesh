// RISK: the safety controls at a glance. Usage against every global limit,
// each strategy's mode (PAPER, ARMED until 15:30, PAUSED) and own limits,
// the kill switch and the live-trading master switch, recent alerts, and
// the typed confirmation a riskier action is waiting for.
//
// Everything is driven from the command bar (RISK SET, ARM, DISARM, LIVE);
// the confirmation box takes the phrase, Enter sends it, Esc cancels.

import { type Component, createEffect, createMemo, createSignal, For, on, Show } from "solid-js";
import { Badge } from "../../components/Badge";
import { DataPanel } from "../../components/DataPanel";
import { type CellTone, type Column, DataTable } from "../../components/DataTable";
import { DISCLAIMER, isLiveAction } from "../../data/disclaimer";
import { createPoll } from "../../data/poll";
import { alerts, answerChallenge, cancelChallenge, challenge, changed } from "../../data/risk";
import { inTauri, type RiskStatusDto, riskStatus, type StrategyRiskDto } from "../../ipc";
import { formatPrice } from "../../lib/format";
import { formatIstHm } from "../../lib/time";
import "./risk.css";

const RISK_REFRESH_MS = 2_000;

/** One global limit and how much of it is used. */
export interface UsageRow {
  limit: string;
  /** What is used now, or null for a per-order limit. */
  used: number | null;
  cap: number | null;
  /** Shown instead of the numbers (products, window). */
  text?: string;
  money?: boolean;
}

/** The usage table's rows, from the status. */
export function usageRows(s: RiskStatusDto): UsageRow[] {
  const l = s.limits;
  const u = s.usage;
  return [
    { limit: "Daily loss", used: Math.max(0, -u.dayPnl), cap: l.maxDailyLoss, money: true },
    { limit: "Open positions", used: u.openPositions, cap: l.maxOpenPositions },
    { limit: "Orders today", used: u.ordersToday, cap: l.maxOrdersPerDay },
    { limit: "Orders / minute", used: u.ordersLastMinute, cap: l.maxOrdersPerMinute },
    {
      limit: "Exposure / symbol",
      used: u.largestExposure,
      cap: l.maxSymbolExposure,
      money: true,
    },
    { limit: "Order quantity", used: null, cap: l.maxOrderQuantity },
    { limit: "Order value", used: null, cap: l.maxOrderValue, money: true },
    { limit: "Price band", used: null, cap: null, text: `${l.priceBandPct}% of LTP` },
    { limit: "Products", used: null, cap: null, text: l.products.join(", ").toUpperCase() },
    {
      limit: "Trading window",
      used: null,
      cap: null,
      text: l.tradingWindow ? `${l.tradingWindow} IST` : "any time",
    },
  ];
}

/** Percent of the cap used, when it means something. */
export function usedPct(r: UsageRow): number | null {
  if (r.used === null || r.cap === null || r.cap <= 0) return null;
  return (r.used / r.cap) * 100;
}

function usageTone(r: UsageRow): CellTone | undefined {
  const pct = usedPct(r);
  if (pct === null) return undefined;
  if (pct >= 100) return "down";
  if (pct >= 80) return "accent";
  return undefined;
}

const num = (v: number, money?: boolean) => (money ? formatPrice(v) : String(v));

const USAGE_COLUMNS: readonly Column<UsageRow>[] = [
  { id: "limit", header: "Limit", width: "20ch", value: (r) => r.limit },
  {
    id: "used",
    header: "Used",
    width: "14ch",
    align: "right",
    value: (r) => (r.used === null ? "" : num(r.used, r.money)),
    tone: usageTone,
  },
  {
    id: "cap",
    header: "Cap",
    width: "16ch",
    align: "right",
    value: (r) => r.text ?? (r.cap === null ? "" : num(r.cap, r.money)),
  },
  {
    id: "pct",
    header: "%",
    width: "8ch",
    align: "right",
    value: (r) => {
      const pct = usedPct(r);
      return pct === null ? "" : `${pct.toFixed(0)}%`;
    },
    tone: usageTone,
  },
];

function modeBadge(s: StrategyRiskDto) {
  if (s.mode === "ARMED") return <Badge tone="down">ARMED</Badge>;
  if (s.mode === "PAUSED") return <Badge tone="stale">PAUSED</Badge>;
  return <Badge tone="paper">PAPER</Badge>;
}

const STRATEGY_COLUMNS: readonly Column<StrategyRiskDto>[] = [
  { id: "name", header: "Strategy", width: "18ch", value: (s) => s.name, tone: () => "accent" },
  { id: "mode", header: "Mode", width: "9ch", value: (s) => s.mode, render: modeBadge },
  {
    id: "run",
    header: "Running",
    width: "8ch",
    value: (s) => (s.running ? "yes" : "no"),
    tone: (s) => (s.running ? undefined : "dim"),
  },
  {
    id: "until",
    header: "Armed until",
    width: "12ch",
    value: (s) => (s.armedUntil === null ? "" : `${formatIstHm(s.armedUntil)} IST`),
  },
  { id: "feed", header: "Feed", width: "9ch", value: (s) => s.feed ?? "" },
  {
    id: "limits",
    header: "Own limits",
    width: "22ch",
    value: (s) =>
      s.limits === null
        ? "none (cannot arm)"
        : `loss ₹${formatPrice(s.limits.maxDailyLoss, 0)} · qty ${s.limits.maxOrderQuantity}`,
    tone: (s) => (s.limits === null ? "dim" : undefined),
  },
  { id: "why", header: "Paused because", value: (s) => s.paused ?? "", tone: () => "dim" },
];

/** The typed confirmation a riskier action waits on. */
const ConfirmBox: Component = () => {
  const [typed, setTyped] = createSignal("");
  const [error, setError] = createSignal<string>();
  let input: HTMLInputElement | undefined;

  // A new challenge starts empty and takes the focus.
  createEffect(
    on(challenge, (c) => {
      setTyped("");
      setError(undefined);
      if (c) queueMicrotask(() => input?.focus());
    }),
  );

  const submit = async (e: Event) => {
    e.preventDefault();
    setError(await answerChallenge(typed()));
  };

  return (
    <Show when={challenge()}>
      {(c) => (
        <form class="risk__confirm" onSubmit={submit} aria-label="Confirm a riskier action">
          <label for="risk-confirm">
            To {c().action}, type <code>{c().phrase}</code> and press Enter. Esc cancels.
          </label>
          <Show when={isLiveAction(c().action)}>
            <p class="risk__disclaimer">{DISCLAIMER}</p>
          </Show>
          <input
            id="risk-confirm"
            ref={input}
            class="risk__confirm-input"
            autocomplete="off"
            spellcheck={false}
            value={typed()}
            onInput={(e) => setTyped(e.currentTarget.value.toUpperCase())}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault();
                cancelChallenge();
              }
            }}
          />
          <Show when={error()}>
            <p class="view__error" role="alert">
              {error()}
            </p>
          </Show>
        </form>
      )}
    </Show>
  );
};

/** `RISK`: limits, usage, modes, switches and alerts. */
export const RiskView: Component = () => {
  const status = createPoll(
    () => inTauri(),
    () => riskStatus(),
    { everyMs: () => RISK_REFRESH_MS },
  );
  // An action or a kill-switch event: refetch now.
  createEffect(on(changed, () => status.refresh(), { defer: true }));

  const s = () => status.data();
  const usage = createMemo(() => (s() ? usageRows(s() as RiskStatusDto) : []));

  const emptyText = () => {
    if (!inTauri()) return "The risk controls are available in the Nivesh app.";
    if (status.error() && !s()) return `${status.error()}. Retrying.`;
    return undefined;
  };

  return (
    <div class="view risk" data-view="risk">
      <DataPanel provenance={null} title="RISK">
        <div class="risk__switches">
          <Show when={s()?.kill.engaged} fallback={<Badge tone="neutral">KILL SWITCH OFF</Badge>}>
            <Badge tone="down" title={s()?.kill.detail}>
              KILL SWITCH ENGAGED
            </Badge>
          </Show>
          <Show when={s()?.liveEnabled} fallback={<Badge tone="paper">PAPER ONLY</Badge>}>
            <Badge tone="down">LIVE TRADING ON</Badge>
          </Show>
          <Show when={s() && !s()?.liveRoute}>
            <Badge tone="neutral">NO LIVE ROUTE</Badge>
          </Show>
        </div>
        <Show when={s()?.kill.engaged}>
          <p class="risk__kill" role="status">
            Kill switch: {s()?.kill.detail}. New orders are blocked.
          </p>
        </Show>
        <ConfirmBox />
        <Show when={status.data() && status.error()}>
          <p class="view__error" role="alert">
            Refresh failed: {status.error()}. Showing the last data.
          </p>
        </Show>
        <DataTable
          label="Usage against the global limits"
          columns={USAGE_COLUMNS}
          rows={usage()}
          emptyText={emptyText()}
        />
        <Show when={(s()?.strategies.length ?? 0) > 0}>
          <DataTable label="Strategies" columns={STRATEGY_COLUMNS} rows={s()?.strategies ?? []} />
        </Show>
        <Show when={alerts().length > 0}>
          <ul class="risk__alerts" aria-label="Recent risk alerts">
            <For each={alerts()}>
              {(a) => (
                <li class={`risk__alert risk__alert--${a.level}`}>
                  {formatIstHm(a.at)} {a.message}
                </li>
              )}
            </For>
          </ul>
        </Show>
        <p class="strat__hint">
          RISK SET &lt;QTY|VALUE|EXPOSURE|POSITIONS|OPM|OPD|LOSS|BAND&gt; &lt;N&gt; [FOR
          &lt;NAME&gt;] · RISK CLEAR &lt;NAME&gt; · LIVE ON|OFF · ARM &lt;NAME&gt; · DISARM
          &lt;NAME&gt; · tightening applies at once; loosening, LIVE ON and ARM ask for a typed
          phrase. Arming needs LIVE data, the strategy's own limits and a connected broker that can
          trade, and ends at 15:30 IST.
        </p>
      </DataPanel>
    </div>
  );
};
