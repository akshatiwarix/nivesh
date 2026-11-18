// What a view can ask of its panel, and how keyboard shortcuts reach it.
import { type Accessor, createContext, onCleanup, useContext } from "solid-js";
import type { View } from "./types";

/** View-local keyboard operations (see the keymap). */
export type ViewOp =
  | "up"
  | "down"
  | "open"
  | "remove"
  | "moveUp"
  | "moveDown"
  | "prevList"
  | "nextList";

export type ViewOpHandler = (op: ViewOp) => void;

// One handler per panel, registered while a view that takes keys is mounted.
const handlers = new Map<number, ViewOpHandler>();

/** Delivers `op` to the view in `panel`. Returns false if it takes no keys. */
export function sendViewOp(panel: number, op: ViewOp): boolean {
  const handler = handlers.get(panel);
  if (!handler) return false;
  handler(op);
  return true;
}

export interface PanelContextValue {
  /** 0-based panel index. */
  panel: number;
  /** Opens a view in this panel (on top of the current one). */
  open: (view: View) => void;
  focused: Accessor<boolean>;
}

export const PanelContext = createContext<PanelContextValue>();

/** The enclosing panel. Falls back to a detached no-op outside a panel (tests). */
export function usePanel(): PanelContextValue {
  return useContext(PanelContext) ?? { panel: -1, open: () => {}, focused: () => false };
}

/** Receives this panel's view-local keys while the calling view is mounted. */
export function useViewOps(handler: ViewOpHandler): void {
  const { panel } = usePanel();
  if (panel < 0) return;
  handlers.set(panel, handler);
  onCleanup(() => {
    if (handlers.get(panel) === handler) handlers.delete(panel);
  });
}
