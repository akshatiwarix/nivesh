// The always-visible safety controls: the kill switch in the status bar
// (one click or Ctrl+Shift+K engages it) and a red LIVE banner whenever
// live trading is switched on or any strategy is armed.

import { type Component, createEffect, createMemo, For, on, Show } from "solid-js";
import { createPoll } from "../data/poll";
import { changed, engageKill, kill, setKill } from "../data/risk";
import { inTauri, riskStatus } from "../ipc";
import { formatIstHm } from "../lib/time";

const STATUS_REFRESH_MS = 5_000;

/** Keeps the kill switch and the armed strategies in view. */
export function createSafetyStatus() {
  const status = createPoll(
    () => inTauri(),
    () => riskStatus(),
    { everyMs: () => STATUS_REFRESH_MS },
  );
  createEffect(on(changed, () => status.refresh(), { defer: true }));
  createEffect(() => {
    const s = status.data();
    if (s) setKill(s.kill);
  });
  return status;
}

/** Status bar: the kill switch, always one click away. */
export const KillControl: Component = () => (
  <Show
    when={kill()?.engaged}
    fallback={
      <button
        type="button"
        class="kill kill--ready"
        title="Engage the kill switch: block new orders, cancel open ones, disarm all"
        onClick={() => void engageKill()}
      >
        KILL ⌃⇧K
      </button>
    }
  >
    <span class="kill kill--engaged" role="status" title={kill()?.detail}>
      KILL SWITCH ENGAGED · KILL RESET
    </span>
  </Show>
);

export interface LiveBannerProps {
  liveEnabled: boolean;
  /** Armed strategies and when each expires (UTC ms). */
  armed: readonly { name: string; until: number | null }[];
}

/** The red banner over the panels while anything can trade for real. */
export const LiveBanner: Component<LiveBannerProps> = (props) => (
  <div class="shell__live">
    <Show when={props.liveEnabled || props.armed.length > 0}>
      <div class="live-banner" role="alert">
        <strong>LIVE</strong>
        <Show
          when={props.armed.length > 0}
          fallback={<span>Live trading is switched on. No strategy is armed.</span>}
        >
          <For each={props.armed}>
            {(a) => (
              <span>
                {a.name} armed{a.until === null ? "" : ` until ${formatIstHm(a.until)} IST`}
              </span>
            )}
          </For>
        </Show>
        <span class="live-banner__hint">Ctrl+Shift+K stops everything · DISARM &lt;NAME&gt;</span>
      </div>
    </Show>
  </div>
);

/** The banner fed from the safety status. */
export const SafetyBanner: Component<{ status: ReturnType<typeof createSafetyStatus> }> = (
  props,
) => {
  const armed = createMemo(() =>
    (props.status.data()?.strategies ?? [])
      .filter((s) => s.mode === "ARMED")
      .map((s) => ({ name: s.name, until: s.armedUntil })),
  );
  return <LiveBanner liveEnabled={props.status.data()?.liveEnabled ?? false} armed={armed()} />;
};
