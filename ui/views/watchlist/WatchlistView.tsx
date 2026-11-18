import { type Component, createMemo, createSignal, For, onMount, Show } from "solid-js";
import { DataPanel } from "../../components/DataPanel";
import { type Column, DataTable } from "../../components/DataTable";
import { ProvenanceBadge } from "../../components/ProvenanceBadge";
import { createFeed, quoteKey } from "../../data/feed";
import { watchlistStore } from "../../data/watchlists";
import { inTauri, type TopicDto, type WatchItemDto } from "../../ipc";
import { changeTone, formatCount, formatMove, formatMovePct, formatPrice } from "../../lib/format";
import { usePanel, useViewOps } from "../context";
import { createSelection } from "../selection";
import "./WatchlistView.css";

/** Same set of instruments, whatever the order: reordering must not resubscribe. */
function sameTopics(a: readonly TopicDto[], b: readonly TopicDto[]): boolean {
  const key = (t: TopicDto) => JSON.stringify(t);
  const [ka, kb] = [a.map(key).sort(), b.map(key).sort()];
  return ka.length === kb.length && ka.every((k, i) => k === kb[i]);
}

/** WL / F3: the active watchlist with live quotes. */
export const WatchlistView: Component = () => {
  const panel = usePanel();
  const store = watchlistStore;
  const [problem, setProblem] = createSignal<string>();

  const items = (): readonly WatchItemDto[] => store.activeList()?.items ?? [];
  const topics = createMemo<TopicDto[]>(
    () => items().map((i) => ({ kind: "quote", exchange: i.exchange, symbol: i.symbol })),
    [],
    { equals: sameTopics },
  );
  const feed = createFeed(topics);
  const quoteOf = (i: WatchItemDto) => feed.state.quotes[quoteKey(i.exchange, i.symbol)];
  const selection = createSelection(() => items().length);

  onMount(() => {
    if (inTauri()) store.load().catch((e: unknown) => setProblem(String(e)));
  });

  /** Runs a store action, showing any failure inline. */
  const attempt = (work: () => Promise<void>) => {
    setProblem(undefined);
    work().catch((e: unknown) => setProblem(e instanceof Error ? e.message : String(e)));
  };

  useViewOps((op) => {
    const item = items()[selection.index()];
    switch (op) {
      case "up":
        selection.move(-1);
        return;
      case "down":
        selection.move(1);
        return;
      case "open":
        if (item)
          panel.open({
            kind: "quote",
            exchange: item.exchange as "NSE" | "BSE",
            symbol: item.symbol,
          });
        return;
      case "remove":
        if (item) attempt(() => store.remove([item]));
        return;
      case "moveUp":
      case "moveDown": {
        if (!item) return;
        const delta = op === "moveUp" ? -1 : 1;
        selection.move(delta);
        attempt(() => store.move(item, delta));
        return;
      }
      case "prevList":
        attempt(() => store.cycle(-1));
        return;
      case "nextList":
        attempt(() => store.cycle(1));
        return;
    }
  });

  const columns: readonly Column<WatchItemDto>[] = [
    {
      id: "symbol",
      header: "Symbol",
      width: "16ch",
      value: (i) => (i.exchange === "NSE" ? i.symbol : `${i.exchange}:${i.symbol}`),
      tone: () => "accent",
    },
    { id: "name", header: "Name", value: (i) => quoteOf(i)?.quote.name ?? "", tone: () => "dim" },
    {
      id: "ltp",
      header: "LTP",
      width: "12ch",
      align: "right",
      value: (i) => formatPrice(quoteOf(i)?.quote.ltp),
    },
    {
      id: "chg",
      header: "Chg",
      width: "13ch",
      align: "right",
      value: (i) => formatMove(quoteOf(i)?.quote.change),
      tone: (i) => changeTone(quoteOf(i)?.quote.change),
    },
    {
      id: "pct",
      header: "% Chg",
      width: "13ch",
      align: "right",
      value: (i) => formatMovePct(quoteOf(i)?.quote.changePct),
      tone: (i) => changeTone(quoteOf(i)?.quote.changePct),
    },
    {
      id: "vol",
      header: "Volume",
      width: "13ch",
      align: "right",
      value: (i) => formatCount(quoteOf(i)?.quote.volume),
    },
    {
      id: "src",
      header: "Source",
      value: () => "",
      render: (i) => (
        <Show when={quoteOf(i)} fallback={<span class="dim">—</span>}>
          {(q) => <ProvenanceBadge provenance={q().provenance} />}
        </Show>
      ),
    },
  ];

  const provenance = createMemo(() => {
    const found = items().flatMap((i) => {
      const q = quoteOf(i);
      return q ? [q.provenance] : [];
    });
    return found.length > 0 ? found : null;
  });

  const firstFeedError = () => feed.fatal() ?? Object.values(feed.state.errors).find(Boolean);
  const emptyText = () =>
    !store.state.loaded && inTauri()
      ? "Loading watchlists…"
      : "Empty. Add a symbol with WL ADD <SYMBOL>.";

  return (
    <div class="view wl" data-view="wl">
      <DataPanel provenance={provenance()} title="Watchlist">
        <nav class="wl__lists" aria-label="Watchlists">
          <For each={store.state.lists}>
            {(list, n) => (
              <span
                class="wl__list"
                classList={{ "wl__list--active": list.id === store.state.active }}
              >
                {n() + 1} {list.name} <span class="wl__count">{list.items.length}</span>
              </span>
            )}
          </For>
          <Show when={store.state.lists.length === 0}>
            <span class="view__dim">No watchlists</span>
          </Show>
        </nav>
        <DataTable
          label="Watchlist"
          columns={columns}
          rows={items()}
          selectedIndex={panel.focused() ? selection.index() : undefined}
          onRowMouseDown={selection.set}
          emptyText={emptyText()}
        />
        <Show when={problem()}>
          {(p) => (
            <p class="view__error" role="alert">
              {p()}
            </p>
          )}
        </Show>
        <Show when={firstFeedError()}>
          {(e) => (
            <p class="view__error" role="alert">
              Some quotes could not be refreshed: {e().message}.
            </p>
          )}
        </Show>
        <p class="wl__hint">
          Alt+↑/↓ select · Enter open · Alt+Shift+↑/↓ reorder · Delete remove · Alt+←/→ list · WL
          ADD SYMBOL · WL NEW NAME
        </p>
      </DataPanel>
    </div>
  );
};
