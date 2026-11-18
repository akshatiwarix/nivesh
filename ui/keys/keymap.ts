// Table-driven keymap. HELP renders this table, so the two cannot drift,
// and a later settings screen can remap it by swapping the table.

export type Action =
  | { type: "layout"; layout: 1 | 2 | 4 }
  | { type: "focus"; delta: 1 | -1 }
  | { type: "fkey"; n: number }
  | { type: "help" }
  | { type: "back" };

/** A key chord. `code` is physical (immune to macOS Option remapping); `key` is logical. */
export interface Chord {
  key?: string;
  code?: string;
  ctrl?: boolean;
  alt?: boolean;
  shift?: boolean;
}

export interface Binding {
  id: string;
  /** Display names for each equivalent chord, e.g. ["F1", "Ctrl+1"]. */
  keys: readonly string[];
  chords: readonly Chord[];
  action: Action;
  summary: string;
  /**
   * Whether the binding still fires while a text input has focus. Only
   * chords that can't produce text (F-keys, Ctrl/Alt combos) are safe.
   */
  inInput: boolean;
}

const FKEYS: readonly Binding[] = Array.from({ length: 10 }, (_, i): Binding => {
  const n = i + 1;
  const digit = n === 10 ? "0" : String(n);
  return {
    id: `fkey.${n}`,
    keys: [`F${n}`, `Ctrl+${digit}`],
    chords: [{ key: `F${n}` }, { code: `Digit${digit}`, ctrl: true }],
    action: { type: "fkey", n },
    summary: `Function key ${n}`,
    inInput: true,
  };
});

export const KEYMAP: readonly Binding[] = [
  ...FKEYS,
  {
    id: "help.question",
    keys: ["?"],
    chords: [{ key: "?" }],
    action: { type: "help" },
    summary: "Open HELP (when the command bar isn't focused)",
    inInput: false,
  },
  {
    id: "focus.next",
    keys: ["Tab"],
    chords: [{ key: "Tab" }],
    action: { type: "focus", delta: 1 },
    summary: "Focus next panel",
    inInput: true,
  },
  {
    id: "focus.prev",
    keys: ["Shift+Tab"],
    chords: [{ key: "Tab", shift: true }],
    action: { type: "focus", delta: -1 },
    summary: "Focus previous panel",
    inInput: true,
  },
  ...([1, 2, 4] as const).map(
    (layout): Binding => ({
      id: `layout.${layout}`,
      keys: [`Alt+${layout}`],
      chords: [{ code: `Digit${layout}`, alt: true }],
      action: { type: "layout", layout },
      summary: `${layout}-panel layout`,
      inInput: true,
    }),
  ),
  {
    id: "back",
    keys: ["Esc"],
    chords: [{ key: "Escape" }],
    action: { type: "back" },
    summary: "Clear the command bar, then go back",
    inInput: false,
  },
];

/** The fields of a KeyboardEvent the keymap looks at. */
export interface KeyLike {
  key: string;
  code: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

function matches(e: KeyLike, c: Chord): boolean {
  if (e.metaKey) return false;
  if (c.key !== undefined && e.key !== c.key) return false;
  if (c.code !== undefined && e.code !== c.code) return false;
  if (e.ctrlKey !== (c.ctrl ?? false)) return false;
  if (e.altKey !== (c.alt ?? false)) return false;
  // `?` needs Shift on most layouts, so Shift only matters when a chord names it.
  if (c.key !== undefined && c.key.length === 1) return true;
  return e.shiftKey === (c.shift ?? false);
}

/**
 * Finds the binding for a key press. `inInput` is true when a text input
 * has focus; bindings that could collide with typing are skipped then.
 */
export function resolveKey(
  e: KeyLike,
  inInput: boolean,
  keymap: readonly Binding[] = KEYMAP,
): Binding | undefined {
  return keymap.find((b) => (!inInput || b.inInput) && b.chords.some((c) => matches(e, c)));
}
