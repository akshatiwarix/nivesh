// Refreshing request/response data for views that are not on the quote stream
// (movers, breadth, sectors, news). Polls while the view is mounted, pauses
// while the window is hidden, keeps the last good data across failures and
// ignores answers to a request the view has since moved on from.
import { type Accessor, createEffect, createSignal, onCleanup } from "solid-js";
import { describeError } from "./errors";

export interface Poll<T> {
  data: Accessor<T | undefined>;
  /** The latest failure, cleared by the next success. */
  error: Accessor<string | undefined>;
  loading: Accessor<boolean>;
  /** Fetches now, outside the schedule. */
  refresh: () => void;
}

export interface PollOptions {
  /** Delay between fetches (ms). May depend on signals, e.g. market open. */
  everyMs: () => number;
}

/**
 * Fetches `fetcher(source())` now and then every `everyMs()`. A `false`
 * source means "nothing to fetch yet". A new source clears the data, so a
 * different universe or interval never shows the previous one's numbers.
 */
export function createPoll<S, T>(
  source: Accessor<S | false>,
  fetcher: (source: S) => Promise<T>,
  options: PollOptions,
): Poll<T> {
  const [data, setData] = createSignal<T>();
  const [error, setError] = createSignal<string>();
  const [loading, setLoading] = createSignal(false);
  let token = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let current: (() => void) | undefined;

  createEffect(() => {
    const s = source();
    const myToken = ++token;
    clearTimeout(timer);
    setData(undefined);
    setError(undefined);
    if (s === false) {
      current = undefined;
      setLoading(false);
      return;
    }
    const run = async () => {
      clearTimeout(timer);
      if (myToken !== token) return;
      if (!document.hidden) {
        setLoading(true);
        try {
          const value = await fetcher(s);
          if (myToken === token) {
            setData(() => value);
            setError(undefined);
          }
        } catch (err) {
          if (myToken === token) setError(describeError(err));
        } finally {
          if (myToken === token) setLoading(false);
        }
      }
      if (myToken === token) timer = setTimeout(run, options.everyMs());
    };
    current = () => void run();
    void run();
  });

  const onVisible = () => {
    // Catch up straight away when the window comes back.
    if (!document.hidden) current?.();
  };
  document.addEventListener("visibilitychange", onVisible);
  onCleanup(() => {
    token++;
    clearTimeout(timer);
    document.removeEventListener("visibilitychange", onVisible);
  });

  return { data, error, loading, refresh: () => current?.() };
}
