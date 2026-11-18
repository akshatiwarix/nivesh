// Typed wrappers for the market-view commands.
import { Channel } from "@tauri-apps/api/core";
import type { BreadthDto } from "./bindings/BreadthDto";
import type { DmaBreadthDto } from "./bindings/DmaBreadthDto";
import type { MoversDto } from "./bindings/MoversDto";
import type { ProgressDto } from "./bindings/ProgressDto";
import type { SectorsDto } from "./bindings/SectorsDto";
import { call } from "./core";

/** Gainers, losers and most active stocks of an index universe (`NIFTY50`, ...). */
export function getMovers(index: string, limit?: number): Promise<MoversDto> {
  return call<MoversDto>("get_movers", { index, limit });
}

/** Advances, declines and unchanged among an index's stocks. */
export function getBreadth(index: string): Promise<BreadthDto> {
  return call<BreadthDto>("get_breadth", { index });
}

/**
 * Share of NIFTY 50 / NIFTY 100 above their 20, 50 and 200-day averages. A cold
 * run takes a minute or so; `onProgress` reports stocks completed.
 */
export function getDmaBreadth(
  universe: string,
  onProgress?: (p: ProgressDto) => void,
): Promise<DmaBreadthDto> {
  const channel = new Channel<ProgressDto>();
  if (onProgress) channel.onmessage = onProgress;
  return call<DmaBreadthDto>("get_dma_breadth", { universe, onProgress: channel });
}

/** Returns of every index over 1D, 1W, 1M and year to date. */
export function getSectors(): Promise<SectorsDto> {
  return call<SectorsDto>("get_sectors");
}
