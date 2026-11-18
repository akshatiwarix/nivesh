// Provider health for the status bar: an initial snapshot, then the backend's
// `provider:status` events.

import { onCleanup, onMount } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { inTauri, onProviderStatus, type ProviderStatusDto, providerStatus } from "../ipc";

export interface HealthState {
  providers: ProviderStatusDto[];
}

export function createProviderHealth() {
  const [state, setState] = createStore<HealthState>({ providers: [] });

  const apply = (status: ProviderStatusDto) => {
    const at = state.providers.findIndex((p) => p.provider === status.provider);
    if (at >= 0) setState("providers", at, reconcile(status));
    else
      setState("providers", (all) =>
        [...all, status].sort((a, b) => a.provider.localeCompare(b.provider)),
      );
  };

  onMount(() => {
    if (!inTauri()) return;
    let closed = false;
    let stop: (() => void) | undefined;
    // Listen first, then fetch, so a change in between isn't missed.
    onProviderStatus(apply).then(
      (unlisten) => {
        if (closed) unlisten();
        else stop = unlisten;
      },
      () => undefined,
    );
    providerStatus().then(
      (all) => {
        // An event that arrived first is newer than the snapshot: keep it.
        if (!closed)
          for (const s of all)
            if (!state.providers.some((p) => p.provider === s.provider)) apply(s);
      },
      () => undefined,
    );
    onCleanup(() => {
      closed = true;
      stop?.();
    });
  });

  return state;
}
