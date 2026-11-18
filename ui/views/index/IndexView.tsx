import { type Component, createMemo, Show } from "solid-js";
import { DataPanel } from "../../components/DataPanel";
import { type Column, DataTable } from "../../components/DataTable";
import { providerName } from "../../components/ProvenanceBadge";
import { createFeed } from "../../data/feed";
import type { QuoteDto } from "../../ipc";
import {
  changeTone,
  formatCount,
  formatMove,
  formatMovePct,
  formatPrice,
  formatRange,
} from "../../lib/format";
import { usePanel, useViewOps } from "../context";
import { createSelection } from "../selection";
import type { View } from "../types";
import "./IndexView.css";

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
  { id: "high", header: "High", width: "11ch", align: "right", value: (q) => formatPrice(q.high) },
  { id: "low", header: "Low", width: "11ch", align: "right", value: (q) => formatPrice(q.low) },
];

/** `NIFTY` and friends: the index level above its constituents, updating in place. */
export const IndexView: Component<{ view: Extract<View, { kind: "index" }> }> = (props) => {
  const panel = usePanel();
  const topic = () => ({ kind: "index", index: props.view.id }) as const;
  const feed = createFeed(() => [topic()]);

  const rows = createMemo<QuoteDto[]>(() =>
    feed.state.constituentOrder.flatMap((symbol) => {
      const q = feed.state.constituents[symbol];
      return q ? [q] : [];
    }),
  );
  const selection = createSelection(() => rows().length);

  useViewOps((op) => {
    if (op === "up") selection.move(-1);
    else if (op === "down") selection.move(1);
    else if (op === "open") {
      const q = rows()[selection.index()];
      if (q) panel.open({ kind: "quote", exchange: q.exchange as "NSE" | "BSE", symbol: q.symbol });
    }
  });

  const error = () => feed.errorFor(topic());
  const emptyText = () => {
    if (!feed.available) return "Market data is available in the Nivesh app.";
    if (feed.state.header) {
      return `Constituents are not available from ${providerName(feed.state.header.provenance.provider)}.`;
    }
    const e = error();
    return e ? `${e.message}. Retrying.` : "Loading…";
  };

  return (
    <div class="view idx" data-view="index">
      <DataPanel provenance={feed.state.header?.provenance ?? null} title={props.view.title}>
        <header class="idx__head">
          <h2 class="view__title">{props.view.title}</h2>
          <Show when={feed.state.header}>
            {(h) => (
              <>
                <span class="idx__last">{formatPrice(h().quote.last)}</span>
                <span classList={{ [changeTone(h().quote.change) ?? ""]: true }}>
                  {formatMove(h().quote.change)} ({formatMovePct(h().quote.changePct)})
                </span>
              </>
            )}
          </Show>
        </header>
        <Show when={feed.state.header}>
          {(h) => (
            <dl class="idx__stats">
              <dt>Open</dt>
              <dd>{formatPrice(h().quote.open)}</dd>
              <dt>High</dt>
              <dd>{formatPrice(h().quote.high)}</dd>
              <dt>Low</dt>
              <dd>{formatPrice(h().quote.low)}</dd>
              <dt>Prev</dt>
              <dd>{formatPrice(h().quote.prevClose)}</dd>
              <dt>52W</dt>
              <dd>{formatRange(h().quote.week52Low, h().quote.week52High)}</dd>
              <Show when={h().quote.advances != null && h().quote.declines != null}>
                <dt>Adv/Dec</dt>
                <dd>
                  <span class="up">{h().quote.advances}</span>/
                  <span class="down">{h().quote.declines}</span>
                </dd>
              </Show>
            </dl>
          )}
        </Show>
        <Show when={feed.state.header && error()}>
          {(e) => (
            <p class="view__error" role="alert">
              Refresh failed: {e().message}. Showing the last data.
            </p>
          )}
        </Show>
        <Show when={feed.state.header?.constituentsInfo}>
          {(info) => (
            <p class="idx__note" data-testid="constituents-asof">
              CONSTITUENTS AS OF {info().asOf}, {info().source}
            </p>
          )}
        </Show>
        <DataTable
          label={`${props.view.title} constituents`}
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
