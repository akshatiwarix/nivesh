import { type Accessor, createSignal, onCleanup } from "solid-js";

/** A signal holding the current time, refreshed once per second. */
export function createClock(now: () => number = Date.now): Accessor<number> {
  const [time, setTime] = createSignal(now());
  const id = setInterval(() => setTime(now()), 1000);
  onCleanup(() => clearInterval(id));
  return time;
}
