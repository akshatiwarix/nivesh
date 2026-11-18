import { type Accessor, createEffect, createSignal } from "solid-js";

/** A keyboard-driven row selection that stays within `count` rows. */
export function createSelection(count: Accessor<number>) {
  const [index, setIndex] = createSignal(0);

  // Rows can disappear (a refresh with fewer rows): keep the selection valid.
  createEffect(() => {
    const n = count();
    if (n === 0) setIndex(0);
    else if (index() >= n) setIndex(n - 1);
  });

  return {
    index,
    set: (i: number) => setIndex(Math.max(0, Math.min(i, count() - 1))),
    move: (delta: number) => setIndex((i) => Math.max(0, Math.min(i + delta, count() - 1))),
  };
}
