// Debounced, ordered instrument search for the command bar.
import { type Accessor, createEffect, createSignal, onCleanup } from "solid-js";
import type { InstrumentDto } from "../ipc";

export type SearchFn = (query: string) => Promise<readonly InstrumentDto[]>;

export const DEBOUNCE_MS = 25;

/**
 * Searches for `query()` after a short pause in typing. Results of superseded
 * searches are dropped, so a slow early response never overwrites a newer one.
 * A failed search just clears the list: suggestions are a convenience.
 */
export function createSymbolSearch(
  query: Accessor<string | undefined>,
  search: SearchFn | undefined,
  debounceMs = DEBOUNCE_MS,
) {
  const [results, setResults] = createSignal<readonly InstrumentDto[]>([]);
  let latest = 0;

  createEffect(() => {
    const q = query();
    const id = ++latest;
    if (!search || q === undefined || q === "") {
      setResults([]);
      return;
    }
    const timer = setTimeout(() => {
      search(q).then(
        (found) => {
          if (id === latest) setResults(found);
        },
        () => {
          if (id === latest) setResults([]);
        },
      );
    }, debounceMs);
    onCleanup(() => clearTimeout(timer));
  });
  onCleanup(() => {
    latest++;
  });

  return results;
}
