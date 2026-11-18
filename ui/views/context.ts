// What a view can ask of its panel, and how keyboard shortcuts reach it.
import { type Accessor, createContext, onCleanup, useContext } from "solid-js";
import type { View } from "./types";

/** View-local keyboard operations (see the keymap). */
export type ViewOp =
  | "up"
  | "down"
  | "open"
  | "openBeside"
  | "remove"
  | "moveUp"
  | "moveDown"
  | "prevList"
  | "nextList"
  | "zoomIn"
  | "zoomOut"
  | "panLeft"
  | "panRight"
  | "home"
  | "end"
  | "tabPrev"
  | "tabNext";

export type ViewOpHandler = (op: ViewOp) => void;

// One handler per panel, registered while a view that takes keys is mounted.
const handlers = new Map<number, ViewOpHandler>();

/** Whether the view in `panel` is listening for view keys. */
export function hasViewOps(panel: number): boolean {
  return handlers.has(panel);
}

/** Delivers `op` to the view in `panel`. Returns false if it takes no keys. */
export function sendViewOp(panel: number, op: ViewOp): boolean {
  const handler = handlers.get(panel);
  if (!handler) return false;
  handler(op);
  return true;
}

/** A typed command aimed at a view (`ADD SMA 50` at a chart). Returns an error message. */
export type ViewCommandHandler = (
  name: string,
  args: readonly string[],
) => string | undefined | Promise<string | undefined>;

const commandHandlers = new Map<number, ViewCommandHandler>();

/** Sends a typed command to the view in `panel`; `undefined` if it takes none. */
export function sendViewCommand(
  panel: number,
  name: string,
  args: readonly string[],
): ReturnType<ViewCommandHandler> | "unhandled" {
  const handler = commandHandlers.get(panel);
  return handler ? handler(name, args) : "unhandled";
}

/** Receives typed commands (ADD, RM) for this panel while the calling view is mounted. */
export function useViewCommands(handler: ViewCommandHandler): void {
  const { panel } = usePanel();
  if (panel < 0) return;
  commandHandlers.set(panel, handler);
  onCleanup(() => {
    if (commandHandlers.get(panel) === handler) commandHandlers.delete(panel);
  });
}

export interface PanelContextValue {
  /** 0-based panel index. */
  panel: number;
  /** Opens a view in this panel (on top of the current one). */
  open: (view: View) => void;
  /** Opens a view in the next panel, widening the layout if there is only one. */
  openBeside: (view: View) => void;
  focused: Accessor<boolean>;
}

export const PanelContext = createContext<PanelContextValue>();

/** The enclosing panel. Falls back to a detached no-op outside a panel (tests). */
export function usePanel(): PanelContextValue {
  return (
    useContext(PanelContext) ?? {
      panel: -1,
      open: () => {},
      openBeside: () => {},
      focused: () => false,
    }
  );
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
