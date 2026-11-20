// Typed wrappers for the safety controls: the kill switch, limits, the
// live-trading master switch and arming.
//
// Safer actions apply at once. Riskier ones (resetting the kill switch,
// loosening a limit, switching live trading on, arming) need a phrase the
// user types, which Rust checks: sent without it, they change nothing and
// answer { done: false, confirm: <phrase> }.

import type { KillStatusDto } from "./bindings/KillStatusDto";
import type { RiskActionDto } from "./bindings/RiskActionDto";
import type { RiskStatusDto } from "./bindings/RiskStatusDto";
import { call } from "./core";

export function killStatus(): Promise<KillStatusDto> {
  return call<KillStatusDto>("kill_status", {});
}

/** Engages the kill switch now: no confirmation, by design. */
export function killEngage(): Promise<KillStatusDto> {
  return call<KillStatusDto>("kill_engage", {});
}

/** Resets it; `typed` must be the reset phrase, checked in Rust. */
export function killReset(typed: string): Promise<KillStatusDto> {
  return call<KillStatusDto>("kill_reset", { typed });
}

/** Limits, usage, the switches and every strategy's mode. */
export function riskStatus(): Promise<RiskStatusDto> {
  return call<RiskStatusDto>("risk_status", {});
}

/**
 * Sets one limit, global or (with `strategy`) that strategy's own. Field:
 * QTY VALUE EXPOSURE POSITIONS OPM OPD LOSS BAND; money in rupees, the
 * band in percent.
 */
export function riskSetLimit(
  field: string,
  value: number,
  strategy?: string,
  typed?: string,
): Promise<RiskActionDto> {
  return call<RiskActionDto>("risk_set_limit", {
    strategy: strategy ?? null,
    field,
    value,
    typed: typed ?? null,
  });
}

/** Removes a strategy's own limits (a loosening). */
export function riskClearLimits(strategy: string, typed?: string): Promise<RiskActionDto> {
  return call<RiskActionDto>("risk_clear_limits", { strategy, typed: typed ?? null });
}

/** The live-trading master switch. Off is immediate. */
export function liveSwitch(on: boolean, typed?: string): Promise<RiskActionDto> {
  return call<RiskActionDto>("live_switch", { on, typed: typed ?? null });
}

/** Arms a running strategy until 15:30 IST. */
export function strategyArm(name: string, typed?: string): Promise<RiskActionDto> {
  return call<RiskActionDto>("strategy_arm", { name, typed: typed ?? null });
}

/** Disarms at once; resolves to whether it was armed. */
export function strategyDisarm(name: string): Promise<boolean> {
  return call<boolean>("strategy_disarm", { name });
}
