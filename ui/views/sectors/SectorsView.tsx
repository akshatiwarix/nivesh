import { type Component, createMemo, createSignal, Show } from "solid-js";
import { DataPanel } from "../../components/DataPanel";
import { type Column, DataTable } from "../../components/DataTable";
import { Heatmap } from "../../components/Heatmap";
import type { Tile } from "../../components/heatmapLayout";
import { createPoll } from "../../data/poll";
import type { IndexSnapshotDto, ProvenanceDto, SectorRowDto } from "../../ipc";
import { getSectors, indexSnapshot, inTauri } from "../../ipc";
import { arrow, changeTone, formatPct, formatPrice } from "../../lib/format";
import { usePanel, useViewOps } from "../context";
import { createSelection } from "../selection";
import "./SectorsView.css";

const SECTORS_REFRESH_MS = 60_000;
const HEATMAP_REFRESH_MS = 20_000;

const pctCell = (v: number | null) => (v === null ? "—" : `${arrow(v)} ${formatPct(v)}`);

const COLUMNS: readonly Column<SectorRowDto>[] = [
  { id: "index", header: "Index", width: "20ch", value: (r) => r.title, tone: () => "accent" },
  { id: "last", header: "Last", width: "12ch", align: "right", value: (r) => formatPrice(r.last) },
  ...(["d1", "w1", "m1", "ytd"] as const).map(
    (key): Column<SectorRowDto> => ({
      id: key,
      header: { d1: "1D", w1: "1W", m1: "1M", ytd: "YTD" }[key],
      width: "13ch",
      align: "right",
      value: (r) => pctCell(r[key]),
      tone: (r) => changeTone(r[key]),
    }),
  ),
];

/** Heatmap tiles for an index's constituents, in the index's own order.
 * Tile area tracks the last session's traded value when the app has it. */
export function tilesFor(snapshot: IndexSnapshotDto | undefined): Tile[] {
  return (snapshot?.constituents ?? []).map((q) => ({
    id: `${q.exchange}:${q.symbol}`,
    label: q.symbol,
    pct: Number.isFinite(q.changePct) ? q.changePct : null,
    weight: snapshot?.activity?.turnover[q.symbol] ?? null,
  }));
}

/**
 * `SEC` / F6: returns of the sector indices over 1D, 1W, 1M and YTD, with a
 * heatmap of the selected index's constituents by % change. Alt+Up/Down pick
 * the index, `[`/`]` move through the heatmap, Enter opens the highlighted
 * stock (or the index when no tile is highlighted), Alt+Enter does so in the
 * next panel.
 */
export const SectorsView: Component = () => {
  const panel = usePanel();
  const sectors = createPoll(
    () => inTauri(),
    () => getSectors(),
    {
      everyMs: () => SECTORS_REFRESH_MS,
    },
  );
  const rows = createMemo(() => sectors.data()?.rows ?? []);
  const selection = createSelection(() => rows().length);
  const [tile, setTile] = createSignal<number>();
  const row = () => rows()[selection.index()];

  const heat = createPoll(
    () => {
      const r = row();
      return inTauri() && r ? r.index : false;
    },
    (index) => indexSnapshot(index),
    { everyMs: () => HEATMAP_REFRESH_MS },
  );
  const tiles = createMemo(() => tilesFor(heat.data()));

  const open = (beside: boolean) => {
    const t = tile();
    const q = t === undefined ? undefined : heat.data()?.constituents[t];
    const view = q
      ? ({ kind: "quote", exchange: q.exchange as "NSE" | "BSE", symbol: q.symbol } as const)
      : row()
        ? ({ kind: "index", id: row()?.index ?? "", title: row()?.title ?? "" } as const)
        : undefined;
    if (!view) return;
    if (beside) panel.openBeside(view);
    else panel.open(view);
  };

  useViewOps((op) => {
    switch (op) {
      case "up":
      case "down":
        selection.move(op === "up" ? -1 : 1);
        setTile(undefined);
        break;
      case "tabPrev":
      case "tabNext": {
        const n = tiles().length;
        if (n === 0) break;
        const d = op === "tabNext" ? 1 : -1;
        setTile((t) => (t === undefined ? (d > 0 ? 0 : n - 1) : (t + d + n) % n));
        break;
      }
      case "open":
        open(false);
        break;
      case "openBeside":
        open(true);
        break;
    }
  });

  const provenance = createMemo<ProvenanceDto[] | null>(() => {
    const a = sectors.data()?.provenance;
    const b = heat.data()?.provenance;
    const list = [a, b].filter((p): p is ProvenanceDto => p !== undefined);
    return list.length > 0 ? list : null;
  });

  const tableEmpty = () => {
    if (!inTauri()) return "Market data is available in the Nivesh app.";
    if (sectors.error() && !sectors.data()) return `${sectors.error()}. Retrying.`;
    return "Loading…";
  };
  const heatEmpty = () => {
    if (!row()) return undefined;
    if (heat.error() && !heat.data()) return `${heat.error()}. Retrying.`;
    if (!heat.data()) return "Loading constituents…";
    if (tiles().length === 0) return `No constituents are available for ${row()?.title}.`;
    return undefined;
  };

  return (
    <div class="view sec" data-view="sectors">
      <DataPanel provenance={provenance()} title="SECTORS">
        <Show when={sectors.data() && sectors.error()}>
          <p class="view__error" role="alert">
            Refresh failed: {sectors.error()}. Showing the last data.
          </p>
        </Show>
        <DataTable
          label="Index returns"
          columns={COLUMNS}
          rows={rows()}
          selectedIndex={panel.focused() ? selection.index() : undefined}
          onRowMouseDown={(i) => {
            selection.set(i);
            setTile(undefined);
          }}
          emptyText={tableEmpty()}
        />
        <Show when={row()}>
          <p class="sec__head">
            {row()?.title} CONSTITUENTS BY % CHANGE
            <Show when={heat.data()?.activity}>
              {(a) => <span class="sec__eod"> · SIZE = TURNOVER (EOD {a().asOf})</span>}
            </Show>
            <span class="sec__hint"> [ ] tile · Enter open · Alt+Up/Down index</span>
          </p>
          <Show when={heatEmpty()}>
            {(text) => (
              <p class="sec__msg" role="status">
                {text()}
              </p>
            )}
          </Show>
          <Heatmap
            tiles={tiles()}
            selected={tile()}
            label={`${row()?.title} constituents by percent change`}
            onSelect={setTile}
            onOpen={(i) => {
              setTile(i);
              open(false);
            }}
          />
        </Show>
      </DataPanel>
    </div>
  );
};
