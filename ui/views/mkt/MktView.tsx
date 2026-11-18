import { type Component, createMemo, Show } from "solid-js";
import { type Column, DataTable } from "../../components/DataTable";
import { ProvenanceBadge } from "../../components/ProvenanceBadge";
import { createFeed } from "../../data/feed";
import type { SourcedIndexQuoteDto } from "../../ipc";
import { changeTone, formatCount, formatMove, formatMovePct, formatPrice } from "../../lib/format";
import { usePanel, useViewOps } from "../context";
import { createSelection } from "../selection";
import "./MktView.css";

type Row = SourcedIndexQuoteDto;

const COLUMNS: readonly Column<Row>[] = [
  {
    id: "index",
    header: "Index",
    width: "20ch",
    value: (r) => r.quote.title,
    tone: () => "accent",
  },
  {
    id: "last",
    header: "Last",
    width: "12ch",
    align: "right",
    value: (r) => formatPrice(r.quote.last),
  },
  {
    id: "chg",
    header: "Chg",
    width: "14ch",
    align: "right",
    value: (r) => formatMove(r.quote.change),
    tone: (r) => changeTone(r.quote.change),
  },
  {
    id: "pct",
    header: "% Chg",
    width: "13ch",
    align: "right",
    value: (r) => formatMovePct(r.quote.changePct),
    tone: (r) => changeTone(r.quote.changePct),
  },
  {
    id: "high",
    header: "High",
    width: "12ch",
    align: "right",
    value: (r) => formatPrice(r.quote.high),
  },
  {
    id: "low",
    header: "Low",
    width: "12ch",
    align: "right",
    value: (r) => formatPrice(r.quote.low),
  },
  {
    id: "adv",
    header: "Adv/Dec",
    width: "10ch",
    align: "right",
    value: (r) =>
      r.quote.advances == null || r.quote.declines == null
        ? "—"
        : `${formatCount(r.quote.advances)}/${formatCount(r.quote.declines)}`,
  },
  {
    id: "src",
    header: "Source",
    value: () => "",
    render: (r) => <ProvenanceBadge provenance={r.provenance} />,
  },
];

/** MKT / F2: the index dashboard. */
export const MktView: Component = () => {
  const panel = usePanel();
  const topic = { kind: "allIndices" } as const;
  const feed = createFeed(() => [topic]);

  const rows = createMemo<Row[]>(() =>
    feed.state.indexOrder.flatMap((code) => {
      const row = feed.state.indexRows[code];
      return row ? [row] : [];
    }),
  );
  const selection = createSelection(() => rows().length);

  const openSelected = () => {
    const row = rows()[selection.index()];
    if (row) panel.open({ kind: "index", id: row.quote.index, title: row.quote.title });
  };
  useViewOps((op) => {
    if (op === "up") selection.move(-1);
    else if (op === "down") selection.move(1);
    else if (op === "open") openSelected();
  });

  const error = () => feed.errorFor(topic);
  const emptyText = () => {
    if (!feed.available) return "Market data is available in the Nivesh app.";
    const e = error();
    return e ? `Indices unavailable: ${e.message}. Retrying.` : "Loading indices…";
  };

  return (
    <div class="view mkt" data-view="mkt">
      <DataTable
        label="Indices"
        columns={COLUMNS}
        rows={rows()}
        selectedIndex={panel.focused() ? selection.index() : undefined}
        onRowMouseDown={selection.set}
        emptyText={emptyText()}
      />
      <Show when={rows().length > 0 && error()}>
        {(e) => (
          <p class="view__error" role="alert">
            Refresh failed: {e().message}. Showing the last data.
          </p>
        )}
      </Show>
    </div>
  );
};
