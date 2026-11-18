import { onCleanup, onMount } from "solid-js";
import { inTauri, setWindowVisible } from "../ipc";

/**
 * Tells the backend when the window is hidden or shown again, so it stops
 * polling quotes nobody can see. Best effort: a failure just means polling
 * carries on.
 */
export function createVisibilityReporter(): void {
  onMount(() => {
    if (!inTauri()) return;
    const report = () => {
      setWindowVisible(!document.hidden).catch(() => undefined);
    };
    document.addEventListener("visibilitychange", report);
    onCleanup(() => document.removeEventListener("visibilitychange", report));
  });
}
