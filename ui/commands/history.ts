// Command history with shell-style ↑/↓ navigation. Pure state, no DOM.

export const MAX_HISTORY = 100;

export interface History {
  /** Oldest first. */
  entries(): readonly string[];
  /** Records a submitted command and resets navigation. */
  push(entry: string): void;
  /** Moves to the previous (older) entry. `draft` is what is currently typed. */
  prev(draft: string): string | undefined;
  /** Moves to the next (newer) entry, ending at the saved draft. */
  next(): string | undefined;
  /** Forgets the navigation position (call when the user edits the text). */
  reset(): void;
}

export function createHistory(initial: readonly string[] = [], onChange?: (e: string[]) => void) {
  let items = initial.filter((s) => s.trim() !== "").slice(-MAX_HISTORY);
  /** Index into `items` while navigating, or items.length when not. */
  let cursor = items.length;
  let draft = "";

  const history: History = {
    entries: () => items,
    push(entry) {
      const e = entry.trim();
      if (e !== "" && items[items.length - 1] !== e) {
        items = [...items, e].slice(-MAX_HISTORY);
        onChange?.(items);
      }
      cursor = items.length;
      draft = "";
    },
    prev(current) {
      if (items.length === 0 || cursor === 0) return undefined;
      if (cursor === items.length) draft = current;
      cursor -= 1;
      return items[cursor];
    },
    next() {
      if (cursor >= items.length) return undefined;
      cursor += 1;
      return cursor === items.length ? draft : items[cursor];
    },
    reset() {
      cursor = items.length;
    },
  };
  return history;
}
