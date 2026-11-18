import { createStore } from "solid-js/store";

/** Number of visible panels. */
export type LayoutKind = 1 | 2 | 4;

export const LAYOUTS: readonly LayoutKind[] = [1, 2, 4];

/** Panels that always exist; a layout shows the first N of them. */
export const PANEL_COUNT = 4;

export interface WorkspaceState {
  layout: LayoutKind;
  /** Index of the focused panel, always < layout. */
  focused: number;
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
  const [state, setState] = createStore<WorkspaceState>({ layout, focused });

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
