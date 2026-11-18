// Watchlist state shared by the WL view and the WL command. The backend owns
// the data (SQLite); every action returns the full new state, which replaces
// ours, so the UI can't drift from what is stored.
import { createRoot } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import type { WatchItemDto, WatchlistDto, WatchlistsDto } from "../ipc";
import * as api from "../ipc";

export interface WatchlistState {
  loaded: boolean;
  lists: WatchlistDto[];
  active: number;
}

/** The subset of the ipc module the store uses (so tests can supply their own). */
export type WatchlistApi = Pick<
  typeof api,
  | "watchlists"
  | "watchlistAdd"
  | "watchlistRemove"
  | "watchlistMove"
  | "watchlistCreate"
  | "watchlistDelete"
  | "watchlistActivate"
>;

export type ItemRef = Pick<WatchItemDto, "exchange" | "symbol">;

/** `TCS`, `BSE:500325`, `nse:infy` -> exchange + symbol. */
export function parseItemRef(text: string): ItemRef {
  const [first, second] = text.trim().toUpperCase().split(":");
  return second === undefined
    ? { exchange: "NSE", symbol: first ?? "" }
    : { exchange: first ?? "", symbol: second };
}

/** Detaches references from any store proxies. */
const plain = (refs: readonly ItemRef[]): ItemRef[] =>
  refs.map((r) => ({ exchange: r.exchange, symbol: r.symbol }));

export function createWatchlistStore(backend: WatchlistApi = api) {
  const [state, setState] = createStore<WatchlistState>({ loaded: false, lists: [], active: 0 });

  const apply = (dto: WatchlistsDto) => {
    setState("lists", reconcile(dto.lists, { key: "id" }));
    setState({ active: dto.active, loaded: true });
  };

  const activeList = (): WatchlistDto | undefined =>
    state.lists.find((l) => l.id === state.active) ?? state.lists[0];

  /** Finds a list by name (any case) or 1-based position. */
  const findList = (nameOrNumber: string): WatchlistDto | undefined => {
    const text = nameOrNumber.trim().toUpperCase();
    const byName = state.lists.find((l) => l.name === text);
    if (byName) return byName;
    return /^\d+$/.test(text) ? state.lists[Number(text) - 1] : undefined;
  };

  return {
    state,
    activeList,
    findList,
    /** Loads the lists once; later calls are free. */
    async load(force = false) {
      if (state.loaded && !force) return;
      apply(await backend.watchlists());
    },
    async add(refs: readonly ItemRef[]) {
      for (const r of plain(refs)) apply(await backend.watchlistAdd(r.exchange, r.symbol));
    },
    async remove(refs: readonly ItemRef[]) {
      // Copy first: callers often pass rows of the store itself, which change
      // under them as each removal is applied.
      for (const r of plain(refs)) apply(await backend.watchlistRemove(r.exchange, r.symbol));
    },
    async move(ref: ItemRef, delta: number) {
      apply(await backend.watchlistMove(ref.exchange, ref.symbol, delta));
    },
    async create(name: string) {
      apply(await backend.watchlistCreate(name));
    },
    async delete(id: number) {
      apply(await backend.watchlistDelete(id));
    },
    async activate(id: number) {
      apply(await backend.watchlistActivate(id));
    },
    /** Switches to the list `delta` places away, wrapping around. */
    async cycle(delta: 1 | -1) {
      const n = state.lists.length;
      if (n < 2) return;
      const at = Math.max(
        0,
        state.lists.findIndex((l) => l.id === state.active),
      );
      const next = state.lists[(at + delta + n) % n];
      if (next) await this.activate(next.id);
    },
  };
}

export type WatchlistStore = ReturnType<typeof createWatchlistStore>;

/** The app's one store, living for the whole session. */
export const watchlistStore: WatchlistStore = createRoot(() => createWatchlistStore());
