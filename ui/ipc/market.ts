import type { MarketStatusDto } from "./bindings/MarketStatusDto";
import { call } from "./core";

export function marketStatus(): Promise<MarketStatusDto> {
  return call<MarketStatusDto>("market_status");
}
