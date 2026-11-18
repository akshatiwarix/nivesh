import { type Component, createMemo, createSignal, Show } from "solid-js";
import { DataPanel } from "../../components/DataPanel";
import { type Column, DataTable } from "../../components/DataTable";
import { createPoll } from "../../data/poll";
import type { MoversDto, QuoteDto } from "../../ipc";
import { getMovers, inTauri } from "../../ipc";
import { changeTone, formatCount, formatMove, formatMovePct, formatPrice } from "../../lib/format";
import { stepUniverse, UNIVERSES } from "../../lib/universe";
import { usePanel, useViewOps } from "../context";
import { createSelection } from "../selection";
import type { View } from "../types";
import "./MoversView.css";

export const TABS = ["gainers", "losers", "active"] as const;
export type Tab = (typeof TABS)[number];
const TAB_LABEL: Record<Tab, string> = {
  gainers: "GAINERS",
  losers: "LOSERS",
  active: "MOST ACTIVE",
};

/** How often to refetch (the backend caches for 15 s, so this is cheap). */
const REFRESH_MS = 20_000;
const LIMIT = 15;

const COLUMNS: readonly Column<QuoteDto>[] = [
  { id: "symbol", header: "Symbol", width: "14ch", value: (q) => q.symbol, tone: () => "accent" },
  { id: "name", header: "Name", value: (q) => q.name ?? "", tone: () => "dim" },
  { id: "ltp", header: "LTP", width: "12ch", align: "right", value: (q) => formatPrice(q.ltp) },
  {
    id: "chg",
    header: "Chg",
    width: "13ch",
    align: "right",
    value: (q) => formatMove(q.change),
    tone: (q) => changeTone(q.change),
  },
  {
    id: "pct",
    header: "% Chg",
    width: "13ch",
    align: "right",
    value: (q) => formatMovePct(q.changePct),
    tone: (q) => changeTone(q.changePct),
  },
  {
    id: "vol",
    header: "Volume",
    width: "13ch",
    align: "right",
    value: (q) => formatCount(q.volume),
  },
];

export function rowsFor(movers: MoversDto | undefined, tab: Tab): readonly QuoteDto[] {
  if (!movers) return [];
  return tab === "gainers" ? movers.gainers : tab === "losers" ? movers.losers : movers.mostActive;
}

/**
 * `MOV` / F5: top gainers, losers and most active stocks of an index
 * universe. `[`/`]` switch list, Alt+Left/Right switch universe, Alt+Up/Down
 * select, Enter opens the quote, Alt+Enter opens it in the next panel.
 */
export const MoversView: Component<{ view: Extract<View, { kind: "mov" }> }> = (props) => {
  const panel = usePanel();
  const [universe, setUniverse] = createSignal(props.view.index);
  const [tab, setTab] = createSignal<Tab>("gainers");
  const title = () => UNIVERSES.find((u) => u.id === universe())?.title ?? universe();

  const poll = createPoll(
    () => (inTauri() ? universe() : false),
    (u) => getMovers(u, LIMIT),
    {
      everyMs: () => REFRESH_MS,
    },
  );
  const rows = createMemo(() => rowsFor(poll.data(), tab()));
  const selection = createSelection(() => rows().length);

  const selected = () => rows()[selection.index()];
  const quoteView = (q: QuoteDto): View => ({
    kind: "quote",
    exchange: q.exchange as "NSE" | "BSE",
    symbol: q.symbol,
  });

  useViewOps((op) => {
    const step = (d: 1 | -1) =>
      setTab((t) => TABS[(TABS.indexOf(t) + d + TABS.length) % TABS.length] ?? t);
    switch (op) {
      case "up":
        selection.move(-1);
        break;
      case "down":
        selection.move(1);
        break;
      case "tabPrev":
        step(-1);
        break;
      case "tabNext":
        step(1);
        break;
      case "prevList":
        setUniverse((u) => stepUniverse(u, -1).id);
        break;
      case "nextList":
        setUniverse((u) => stepUniverse(u, 1).id);
        break;
      case "open": {
        const q = selected();
        if (q) panel.open(quoteView(q));
        break;
      }
      case "openBeside": {
        const q = selected();
        if (q) panel.openBeside(quoteView(q));
        break;
      }
    }
  });

  const emptyText = () => {
    if (!inTauri()) return "Market data is available in the Nivesh app.";
    if (poll.error() && !poll.data()) return `${poll.error()}. Retrying.`;
    if (!poll.data()) return "Loading…";
    if (tab() === "active") {
      return "No volume from this source. Most active needs NSE's index quotes.";
    }
    return "No stocks in this list.";
  };

  return (
    <div class="view mov" data-view="movers">
      <DataPanel provenance={poll.data()?.provenance ?? null} title={`MOVERS · ${title()}`}>
        <div class="mov__tabs" role="tablist" aria-label="Movers list">
          {TABS.map((t) => (
            <button
              type="button"
              role="tab"
              class="mov__tab"
              classList={{ "mov__tab--on": t === tab() }}
              aria-selected={t === tab()}
              tabIndex={-1}
              onClick={() => setTab(t)}
            >
              {TAB_LABEL[t]}
            </button>
          ))}
          <span class="mov__hint">[ ] list · Alt+←/→ index</span>
        </div>
        <Show when={poll.data() && poll.error()}>
          <p class="view__error" role="alert">
            Refresh failed: {poll.error()}. Showing the last data.
          </p>
        </Show>
        <DataTable
          label={`${TAB_LABEL[tab()]} in ${title()}`}
          columns={COLUMNS}
          rows={rows()}
          selectedIndex={panel.focused() ? selection.index() : undefined}
          onRowMouseDown={selection.set}
          emptyText={emptyText()}
        />
      </DataPanel>
    </div>
  );
};
