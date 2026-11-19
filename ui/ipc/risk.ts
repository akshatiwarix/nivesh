// Typed wrappers for the safety controls: the kill switch.

import type { KillStatusDto } from "./bindings/KillStatusDto";
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
