// Typed wrappers for the market-view commands.
import type { MoversDto } from "./bindings/MoversDto";
import { call } from "./core";

/** Gainers, losers and most active stocks of an index universe (`NIFTY50`, ...). */
export function getMovers(index: string, limit?: number): Promise<MoversDto> {
  return call<MoversDto>("get_movers", { index, limit });
}
