import { type Accessor, createSignal, onCleanup } from "solid-js";

// One shared one-second ticker for everything that shows an age ("12s ago"),
// running only while something is mounted that reads it.
const [now, setNow] = createSignal(Date.now());
let subscribers = 0;
let timer: ReturnType<typeof setInterval> | undefined;

/** The current time (epoch ms), updated every second while the caller lives. */
export function useNow(): Accessor<number> {
  subscribers += 1;
  setNow(Date.now());
  if (timer === undefined) timer = setInterval(() => setNow(Date.now()), 1000);
  onCleanup(() => {
    subscribers -= 1;
    if (subscribers === 0 && timer !== undefined) {
      clearInterval(timer);
      timer = undefined;
    }
  });
  return now;
}
