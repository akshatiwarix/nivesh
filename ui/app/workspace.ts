import { createStore } from "solid-js/store";
import { EMPTY_VIEW, type View } from "../views/types";

/** Number of visible panels. */
export type LayoutKind = 1 | 2 | 4;

export const LAYOUTS: readonly LayoutKind[] = [1, 2, 4];

/** Panels that always exist; a layout shows the first N of them. */
export const PANEL_COUNT = 4;

/** Views kept per panel, so going back never loses more than it should. */
const MAX_STACK = 20;

export interface WorkspaceState {
  layout: LayoutKind;
  /** Index of the focused panel, always < layout. */
  focused: number;
  /** One view stack per panel, bottom first. Never empty. */
  stacks: View[][];
}

export function isLayoutKind(value: unknown): value is LayoutKind {
  return value === 1 || value === 2 || value === 4;
}

/**
 * Layout and focus for the panel grid. Hidden panels keep their state, so
 * going from 4 panels to 1 and back restores what panels 2-4 showed.
 */
export function createWorkspace(initial?: Partial<WorkspaceState>) {
  const layout = isLayoutKind(initial?.layout) ? initial.layout : 1;
  const focused = clampFocus(initial?.focused ?? 0, layout);
  const [state, setState] = createStore<WorkspaceState>({
    layout,
    focused,
    stacks: Array.from({ length: PANEL_COUNT }, () => [EMPTY_VIEW]),
  });

  return {
    state,
    setLayout(next: LayoutKind) {
      setState({ layout: next, focused: clampFocus(state.focused, next) });
    },
    focus(index: number) {
      if (Number.isInteger(index) && index >= 0 && index < state.layout) {
        setState("focused", index);
      }
    },
    /** The view currently shown in a panel. */
    currentView(panel: number): View {
      const stack = state.stacks[panel];
      return stack?.[stack.length - 1] ?? EMPTY_VIEW;
    },
    /** Opens a view in the focused panel. */
    open(view: View) {
      setState("stacks", state.focused, (stack) => [...stack, view].slice(-MAX_STACK));
    },
    /** Pops the focused panel's view. Returns false when already at the bottom. */
    back(): boolean {
      const stack = state.stacks[state.focused];
      if (!stack || stack.length <= 1) return false;
      setState("stacks", state.focused, stack.slice(0, -1));
      return true;
    },
    /** Moves focus by `delta` panels, wrapping around. */
    cycleFocus(delta: 1 | -1) {
      setState("focused", (state.focused + delta + state.layout) % state.layout);
    },
  };
}

export type Workspace = ReturnType<typeof createWorkspace>;

function clampFocus(index: number, layout: LayoutKind): number {
  return Number.isInteger(index) && index >= 0 && index < layout ? index : 0;
}
