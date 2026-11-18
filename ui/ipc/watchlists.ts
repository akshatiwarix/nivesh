import type { WatchlistsDto } from "./bindings/WatchlistsDto";
import { call } from "./core";

export function watchlists(): Promise<WatchlistsDto> {
  return call<WatchlistsDto>("watchlists");
}

export function watchlistCreate(name: string): Promise<WatchlistsDto> {
  return call<WatchlistsDto>("watchlist_create", { name });
}

export function watchlistDelete(id: number): Promise<WatchlistsDto> {
  return call<WatchlistsDto>("watchlist_delete", { id });
}

export function watchlistActivate(id: number): Promise<WatchlistsDto> {
  return call<WatchlistsDto>("watchlist_activate", { id });
}

/** Adds to `listId`, or to the active list when omitted. */
export function watchlistAdd(
  exchange: string,
  symbol: string,
  listId?: number,
): Promise<WatchlistsDto> {
  return call<WatchlistsDto>("watchlist_add", { exchange, symbol, listId });
}

export function watchlistRemove(
  exchange: string,
  symbol: string,
  listId?: number,
): Promise<WatchlistsDto> {
  return call<WatchlistsDto>("watchlist_remove", { exchange, symbol, listId });
}

/** Moves an item `delta` places (negative = up). */
export function watchlistMove(
  exchange: string,
  symbol: string,
  delta: number,
  listId?: number,
): Promise<WatchlistsDto> {
  return call<WatchlistsDto>("watchlist_move", { exchange, symbol, delta, listId });
}
