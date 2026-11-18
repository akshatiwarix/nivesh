// The index universes a market view can be pointed at. Mirrors the catalogue
// in nivesh-core through `commands/registry` (SENSEX is left out: it is a BSE
// index with no constituent list to rank).
import { findIndex, INDICES } from "../commands";

export interface Universe {
  id: string;
  title: string;
}

export const UNIVERSES: readonly Universe[] = INDICES.filter((i) => i.id !== "SENSEX").map((i) => ({
  id: i.id,
  title: i.title,
}));

/** `BANKNIFTY`, `nifty`, `NIFTYIT`... -> a universe, if it is one. */
export function findUniverse(alias: string): Universe | undefined {
  const index = findIndex(alias);
  return index ? UNIVERSES.find((u) => u.id === index.id) : undefined;
}

/** The universe `delta` places away, wrapping around. */
export function stepUniverse(id: string, delta: 1 | -1): Universe {
  const i = UNIVERSES.findIndex((u) => u.id === id);
  const next = (i + delta + UNIVERSES.length) % UNIVERSES.length;
  return UNIVERSES[next] ?? (UNIVERSES[0] as Universe);
}

export const DEFAULT_UNIVERSE: Universe = UNIVERSES[0] as Universe;
