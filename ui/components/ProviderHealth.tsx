import { type Component, For } from "solid-js";
import type { HealthState } from "../data/health";
import type { HealthDto, ProviderStatusDto } from "../ipc";
import { formatAsOf } from "../lib/time";
import { Badge, type BadgeTone } from "./Badge";
import { providerName } from "./ProvenanceBadge";

const TONE: Record<HealthDto, BadgeTone> = { HEALTHY: "up", DEGRADED: "stale", DOWN: "down" };
const WORD: Record<HealthDto, string> = { HEALTHY: "OK", DEGRADED: "SLOW", DOWN: "DOWN" };

function titleFor(s: ProviderStatusDto): string {
  const name = providerName(s.provider);
  const lastOk = s.lastOkMs == null ? "never answered" : `last answered ${formatAsOf(s.lastOkMs)}`;
  switch (s.state) {
    case "HEALTHY":
      return `${name} is answering (${lastOk}).`;
    case "DEGRADED":
      return `${name}: ${s.consecutiveFailures} recent failure(s)${s.lastError ? `, ${s.lastError}` : ""}. Still being tried.`;
    case "DOWN":
      return `${name} is not answering${s.lastError ? ` (${s.lastError})` : ""}; other sources are used until it recovers. ${lastOk}.`;
  }
}

/** One badge per data provider: `NSE OK`, `YAHOO DOWN`. For the status bar. */
export const ProviderHealth: Component<{ health: HealthState }> = (props) => (
  <output class="health" aria-label="Data provider health">
    <For each={props.health.providers}>
      {(s) => (
        <Badge tone={TONE[s.state]} title={titleFor(s)}>
          {providerName(s.provider)} {WORD[s.state]}
        </Badge>
      )}
    </For>
  </output>
);
