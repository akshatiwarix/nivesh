import { createEffect, on, onMount } from "solid-js";
import { createHistory, type History } from "../commands/history";
import { inTauri, loadUiState, saveCommandHistory, saveLayout } from "../ipc";
import { isLayoutKind, type Workspace } from "./workspace";

/**
 * Restores the saved layout and command history on start and writes changes
 * back. Does nothing outside the app (browser, tests without a backend).
 *
 * Nothing is written until the saved state has loaded, so the defaults can't
 * overwrite what the user had.
 */
export function createPersistence(ws: Workspace): { history: History } {
  let ready = false;
  // A failed save is not worth interrupting the user for; the next change retries.
  const ignore = () => {};

  const history = createHistory([], (entries) => {
    if (ready) saveCommandHistory(entries).catch(ignore);
  });

  onMount(() => {
    if (!inTauri()) return;
    loadUiState()
      .then((state) => {
        if (isLayoutKind(state.layout)) ws.setLayout(state.layout);
        history.load(state.history);
      })
      .catch(ignore)
      .finally(() => {
        ready = true;
      });
  });

  createEffect(
    on(
      () => ws.state.layout,
      (layout) => {
        if (ready) saveLayout(layout).catch(ignore);
      },
      { defer: true },
    ),
  );

  return { history };
}
