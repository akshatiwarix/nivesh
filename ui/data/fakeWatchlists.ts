// An in-memory stand-in for the watchlist backend, for tests. It follows the
// same rules as the Rust store: uppercase unique names, no duplicates, the
// last list can't be deleted.
import type { WatchlistDto, WatchlistsDto } from "../ipc";

export function createFakeWatchlistBackend() {
  let nextId = 2;
  let lists: WatchlistDto[] = [{ id: 1, name: "WATCHLIST", items: [] }];
  let active = 1;
  const calls: string[] = [];

  const snap = (): WatchlistsDto => structuredClone({ lists, active });
  const list = (id: number) => {
    const l = lists.find((x) => x.id === id);
    if (!l) throw new Error("not found");
    return l;
  };
  const target = (listId?: number) => list(listId ?? active);

  return {
    calls,
    /** Back to a fresh install: one empty list. */
    reset() {
      nextId = 2;
      lists = [{ id: 1, name: "WATCHLIST", items: [] }];
      active = 1;
      calls.length = 0;
    },
    watchlists: async () => {
      calls.push("watchlists");
      return snap();
    },
    watchlistAdd: async (exchange: string, symbol: string, listId?: number) => {
      calls.push(`add ${exchange}:${symbol}`);
      if (symbol.startsWith("BAD")) throw new Error(`${symbol} is not a listed NSE symbol`);
      const l = target(listId);
      if (!l.items.some((i) => i.exchange === exchange && i.symbol === symbol)) {
        l.items.push({ exchange, symbol });
      }
      return snap();
    },
    watchlistRemove: async (exchange: string, symbol: string, listId?: number) => {
      calls.push(`remove ${exchange}:${symbol}`);
      const l = target(listId);
      l.items = l.items.filter((i) => !(i.exchange === exchange && i.symbol === symbol));
      return snap();
    },
    watchlistMove: async (exchange: string, symbol: string, delta: number, listId?: number) => {
      calls.push(`move ${symbol} ${delta}`);
      const l = target(listId);
      const from = l.items.findIndex((i) => i.exchange === exchange && i.symbol === symbol);
      if (from >= 0) {
        const to = Math.max(0, Math.min(l.items.length - 1, from + delta));
        const [item] = l.items.splice(from, 1);
        if (item) l.items.splice(to, 0, item);
      }
      return snap();
    },
    watchlistCreate: async (name: string) => {
      calls.push(`create ${name}`);
      const clean = name.trim().toUpperCase();
      if (clean === "") throw new Error("a watchlist needs a name");
      if (lists.some((l) => l.name === clean)) {
        throw new Error(`a watchlist named ${clean} already exists`);
      }
      lists.push({ id: nextId, name: clean, items: [] });
      active = nextId++;
      return snap();
    },
    watchlistDelete: async (id: number) => {
      calls.push(`delete ${id}`);
      if (lists.length <= 1) throw new Error("the last watchlist can't be deleted");
      list(id);
      lists = lists.filter((l) => l.id !== id);
      if (active === id) active = lists[0]?.id ?? 0;
      return snap();
    },
    watchlistActivate: async (id: number) => {
      calls.push(`activate ${id}`);
      list(id);
      active = id;
      return snap();
    },
  };
}

export type FakeWatchlistBackend = ReturnType<typeof createFakeWatchlistBackend>;
