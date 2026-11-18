import { type Component, createMemo, createSignal, For, Show } from "solid-js";
import { DataPanel } from "../../components/DataPanel";
import { createPoll } from "../../data/poll";
import type { DmaPeriodDto, ProvenanceDto } from "../../ipc";
import { getBreadth, getDmaBreadth, inTauri } from "../../ipc";
import { formatCount } from "../../lib/format";
import { stepUniverse, UNIVERSES } from "../../lib/universe";
import { useViewOps } from "../context";
import type { View } from "../types";
import "./BreadthView.css";

const BREADTH_REFRESH_MS = 20_000;
/** The sweep is expensive; daily candles barely change within a session. */
const DMA_REFRESH_MS = 15 * 60_000;

export const DMA_UNIVERSES = [
  { id: "NIFTY50", title: "NIFTY 50" },
  { id: "NIFTY100", title: "NIFTY 100" },
] as const;

/** `62.0%` or a dash. */
export const pctText = (v: number | null) => (v === null ? "—" : `${v.toFixed(1)}%`);

const Meter: Component<{ percent: number | null; tone?: "up" | "down" | "amber" }> = (props) => (
  <span class="brd__meter" aria-hidden="true">
    <span
      class={`brd__fill brd__fill--${props.tone ?? "amber"}`}
      style={{ width: `${Math.max(0, Math.min(100, props.percent ?? 0))}%` }}
    />
  </span>
);

/**
 * `BRD`: advances, declines and unchanged for an index, and the share of NIFTY
 * 50 / NIFTY 100 above their 20, 50 and 200-day moving averages. Alt+Left/Right
 * pick the index, `[`/`]` pick NIFTY 50 or 100 for the averages.
 */
export const BreadthView: Component<{ view: Extract<View, { kind: "brd" }> }> = (props) => {
  const [universe, setUniverse] = createSignal(props.view.index);
  const [dmaUniverse, setDmaUniverse] = createSignal<"NIFTY50" | "NIFTY100">("NIFTY50");
  const [progress, setProgress] = createSignal<{ done: number; total: number }>();
  const title = () => UNIVERSES.find((u) => u.id === universe())?.title ?? universe();

  const breadth = createPoll(
    () => (inTauri() ? universe() : false),
    (u) => getBreadth(u),
    {
      everyMs: () => BREADTH_REFRESH_MS,
    },
  );
  const dma = createPoll(
    () => (inTauri() ? dmaUniverse() : false),
    (u) => {
      setProgress(undefined);
      return getDmaBreadth(u, setProgress);
    },
    { everyMs: () => DMA_REFRESH_MS },
  );

  useViewOps((op) => {
    if (op === "prevList") setUniverse((u) => stepUniverse(u, -1).id);
    else if (op === "nextList") setUniverse((u) => stepUniverse(u, 1).id);
    else if (op === "tabPrev" || op === "tabNext") {
      setDmaUniverse((u) => (u === "NIFTY50" ? "NIFTY100" : "NIFTY50"));
    }
  });

  const total = () => {
    const b = breadth.data();
    return b ? b.advances + b.declines + b.unchanged : 0;
  };
  const share = (n: number) => (total() === 0 ? 0 : (n / total()) * 100);

  const provenance = createMemo<ProvenanceDto[] | null>(() => {
    const list = [breadth.data()?.provenance, dma.data()?.provenance].filter(
      (p): p is ProvenanceDto => p !== undefined,
    );
    return list.length > 0 ? list : null;
  });

  const breadthEmpty = () => {
    if (!inTauri()) return "Market data is available in the Nivesh app.";
    if (breadth.error()) return `${breadth.error()}. Retrying.`;
    return "Loading…";
  };
  const dmaStatus = () => {
    if (!inTauri()) return undefined;
    if (dma.error() && !dma.data()) return `${dma.error()}. Retrying.`;
    const p = progress();
    return p && p.total > 0
      ? `Computing from daily candles: ${p.done} of ${p.total} stocks. The first run takes a minute.`
      : "Loading…";
  };

  return (
    <div class="view brd" data-view="breadth">
      <DataPanel provenance={provenance()} title={`BREADTH · ${title()}`}>
        <section class="brd__section" aria-label="Advances and declines">
          <h3 class="brd__h">
            ADVANCES / DECLINES · {title()}
            <span class="brd__hint"> Alt+←/→ index</span>
          </h3>
          <Show when={breadth.data()} fallback={<p class="brd__msg">{breadthEmpty()}</p>}>
            {(b) => (
              <>
                <div class="brd__bar" role="img" aria-label="Advances, unchanged and declines">
                  <span
                    class="brd__seg brd__seg--up"
                    style={{ width: `${share(b().advances)}%` }}
                  />
                  <span
                    class="brd__seg brd__seg--flat"
                    style={{ width: `${share(b().unchanged)}%` }}
                  />
                  <span
                    class="brd__seg brd__seg--down"
                    style={{ width: `${share(b().declines)}%` }}
                  />
                </div>
                <p class="brd__counts">
                  <span class="up">▲ {formatCount(b().advances)} advancing</span>
                  <span class="dim">– {formatCount(b().unchanged)} unchanged</span>
                  <span class="down">▼ {formatCount(b().declines)} declining</span>
                </p>
              </>
            )}
          </Show>
          <Show when={breadth.data() && breadth.error()}>
            <p class="view__error" role="alert">
              Refresh failed: {breadth.error()}. Showing the last data.
            </p>
          </Show>
        </section>

        <section class="brd__section" aria-label="Stocks above moving averages">
          <h3 class="brd__h">
            % ABOVE MOVING AVERAGE · {dmaUniverse() === "NIFTY50" ? "NIFTY 50" : "NIFTY 100"}
            <span class="brd__hint"> [ ] universe</span>
          </h3>
          <Show when={dma.data()} fallback={<p class="brd__msg">{dmaStatus()}</p>}>
            {(d) => (
              <>
                <For each={d().periods}>
                  {(p: DmaPeriodDto) => (
                    <div class="brd__row" data-testid={`dma-${p.period}`}>
                      <span class="brd__label">{p.period}-DAY</span>
                      <Meter percent={p.percent} tone={(p.percent ?? 0) >= 50 ? "up" : "down"} />
                      <span class="brd__pct">{pctText(p.percent)}</span>
                      <span class="brd__of dim">
                        {p.measured === 0 ? "not enough history" : `${p.above} of ${p.measured}`}
                      </span>
                    </div>
                  )}
                </For>
                <p class="brd__note" data-testid="dma-note">
                  Members as of {d().membersAsOf}, {d().membersSource}. Daily closes.
                </p>
              </>
            )}
          </Show>
          <Show when={dma.data() && dma.error()}>
            <p class="view__error" role="alert">
              Refresh failed: {dma.error()}. Showing the last data.
            </p>
          </Show>
        </section>
      </DataPanel>
    </div>
  );
};
