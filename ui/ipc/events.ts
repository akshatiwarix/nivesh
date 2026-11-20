// Backend broadcasts (`domain:event`). Only listening is permitted.
import { listen } from "@tauri-apps/api/event";
import type { KillStatusDto } from "./bindings/KillStatusDto";
import type { MarketStatusDto } from "./bindings/MarketStatusDto";
import type { ProviderStatusDto } from "./bindings/ProviderStatusDto";
import type { RiskAlertDto } from "./bindings/RiskAlertDto";

/** Subscribes to an event; resolves to the function that stops listening. */
async function on<T>(event: string, handler: (payload: T) => void): Promise<() => void> {
  return listen<T>(event, (e) => handler(e.payload));
}

/** A provider's health changed. */
export function onProviderStatus(handler: (s: ProviderStatusDto) => void): Promise<() => void> {
  return on<ProviderStatusDto>("provider:status", handler);
}

/** The NSE session changed (pre-open, open, closing, closed). */
export function onMarketSession(handler: (s: MarketStatusDto) => void): Promise<() => void> {
  return on<MarketStatusDto>("market:session", handler);
}

/** The kill switch engaged or reset (by the user or on its own). */
export function onKillSwitchChanged(handler: (s: KillStatusDto) => void): Promise<() => void> {
  return on<KillStatusDto>("killswitch:changed", handler);
}

/** Something the user must see: a trip, a refused or ambiguous live order. */
export function onRiskAlert(handler: (a: RiskAlertDto) => void): Promise<() => void> {
  return on<RiskAlertDto>("risk:alert", handler);
}
