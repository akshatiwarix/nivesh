// Table-driven keymap. HELP renders this table, so the two cannot drift,
// and a later settings screen can remap it by swapping the table.

import type { ViewOp } from "../views/context";

export type Action =
  | { type: "layout"; layout: 1 | 2 | 4 }
  | { type: "focus"; delta: 1 | -1 }
  | { type: "fkey"; n: number }
  | { type: "help" }
  | { type: "back" }
  | { type: "view"; op: ViewOp }
  | { type: "kill" };

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
  /** With an input focused, fire only while it is empty (e.g. Delete). */
  emptyInputOnly?: boolean;
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

/**
 * Keys that act on the focused panel's view (lists, watchlists). They use
 * modifiers or non-printing keys, so they never collide with typing, and the
 * command bar keeps Up/Down for history.
 */
const VIEW_KEYS: readonly Binding[] = [
  viewKey("up", "Alt+Up", { key: "ArrowUp", alt: true }, "Select the previous row"),
  viewKey("down", "Alt+Down", { key: "ArrowDown", alt: true }, "Select the next row"),
  {
    id: "view.open",
    keys: ["Enter"],
    // Handled by the command bar: Enter on an empty bar.
    chords: [],
    action: { type: "view", op: "open" },
    summary: "Open the selected row (command bar empty)",
    inInput: true,
  },
  viewKey(
    "openBeside",
    "Alt+Enter",
    { key: "Enter", alt: true },
    "Open the selected row in the next panel",
  ),
  {
    ...viewKey(
      "remove",
      "Delete",
      { key: "Delete" },
      "Remove the selected row (command bar empty)",
    ),
    emptyInputOnly: true,
  },
  viewKey(
    "moveUp",
    "Alt+Shift+Up",
    { key: "ArrowUp", alt: true, shift: true },
    "Move the selected row up",
  ),
  viewKey(
    "moveDown",
    "Alt+Shift+Down",
    { key: "ArrowDown", alt: true, shift: true },
    "Move the selected row down",
  ),
  viewKey("prevList", "Alt+Left", { key: "ArrowLeft", alt: true }, "Previous watchlist or index"),
  viewKey("nextList", "Alt+Right", { key: "ArrowRight", alt: true }, "Next watchlist or index"),
  // Chart keys. They only act while the command bar is empty, so typing is
  // never intercepted (no symbol starts with these characters).
  emptyBarKey("zoomIn", "+", [{ key: "+" }, { key: "=" }], "Chart: zoom in"),
  emptyBarKey("zoomOut", "-", [{ key: "-" }, { key: "_" }], "Chart: zoom out"),
  emptyBarKey("panLeft", "Left", [{ key: "ArrowLeft" }], "Chart: pan back in time"),
  emptyBarKey("panRight", "Right", [{ key: "ArrowRight" }], "Chart: pan forward"),
  emptyBarKey("home", "Home", [{ key: "Home" }], "Chart: jump to the oldest bar"),
  emptyBarKey("end", "End", [{ key: "End" }], "Chart: jump to the newest bar"),
  emptyBarKey("tabPrev", "[", [{ key: "[" }], "Chart: finer interval. Movers: previous tab"),
  emptyBarKey("tabNext", "]", [{ key: "]" }], "Chart: coarser interval. Movers: next tab"),
];

function emptyBarKey(op: ViewOp, label: string, chords: Chord[], summary: string): Binding {
  return {
    id: `view.${op}`,
    keys: [label],
    chords,
    action: { type: "view", op },
    summary,
    inInput: true,
    emptyInputOnly: true,
  };
}

function viewKey(op: ViewOp, label: string, chord: Chord, summary: string): Binding {
  return {
    id: `view.${op}`,
    keys: [label],
    chords: [chord],
    action: { type: "view", op },
    summary,
    inInput: true,
  };
}

export const KEYMAP: readonly Binding[] = [
  ...FKEYS,
  {
    id: "kill",
    keys: ["Ctrl+Shift+K"],
    chords: [{ code: "KeyK", ctrl: true, shift: true }],
    action: { type: "kill" },
    summary: "Engage the kill switch: block new orders, cancel open ones, disarm all",
    inInput: true,
  },
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
  ...VIEW_KEYS,
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
 * `inputEmpty` says whether that input has no text.
 */
export function resolveKey(
  e: KeyLike,
  inInput: boolean,
  keymap: readonly Binding[] = KEYMAP,
  inputEmpty = false,
): Binding | undefined {
  return keymap.find(
    (b) =>
      (!inInput || b.inInput) &&
      !(inInput && b.emptyInputOnly && !inputEmpty) &&
      b.chords.some((c) => matches(e, c)),
  );
}
