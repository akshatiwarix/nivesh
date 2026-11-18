// Test helper: pins `Date.now()` so as-of times and ages render the same on
// every day the tests run. Only Date is faked; timers stay real.
import { vi } from "vitest";
import { AS_OF } from "./fixtures";

/** Pins "now" to `offsetMs` after the fixtures' as-of time (default 30 s). */
export function pinClock(offsetMs = 30_000): void {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(AS_OF + offsetMs);
}

export function unpinClock(): void {
  vi.useRealTimers();
}
