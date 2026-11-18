import { type Component, For } from "solid-js";
import type { HealthState } from "../data/health";
import type { HealthDto, ProviderStatusDto } from "../ipc";
import { formatAsOf } from "../lib/time";
import { Badge, type BadgeTone } from "./Badge";
import { providerName } from "./ProvenanceBadge";

const TONE: Record<HealthDto, BadgeTone> = { HEALTHY: "up", DEGRADED: "stale", DOWN: "down" };
const WORD: Record<HealthDto, string> = { HEALTHY: "OK", DEGRADED: "SLOW", DOWN: "DOWN" };

/** Who takes over when a primary source is out (mirrors the backend's routing for quotes). */
const FALLBACK: Record<string, string> = { nse: "yahoo" };

/** The source that is standing in for `s`, if `s` is out and its fallback is still being served. */
function standIn(s: ProviderStatusDto, all: readonly ProviderStatusDto[]): string | undefined {
  if (s.state !== "DOWN") return undefined;
  const to = FALLBACK[s.provider];
  const other = all.find((p) => p.provider === to);
  return other && other.state !== "DOWN" ? providerName(other.provider) : undefined;
}

/** `NSE OK`, `NSE SLOW`, `NSE DOWN`, or `NSE BLOCKED -> YAHOO` when NSE refuses us and Yahoo covers. */
export function healthLabel(s: ProviderStatusDto, all: readonly ProviderStatusDto[]): string {
  const word = s.state === "DOWN" && s.blocked ? "BLOCKED" : WORD[s.state];
  const by = standIn(s, all);
  return `${providerName(s.provider)} ${word}${by ? ` -> ${by}` : ""}`;
}

function titleFor(s: ProviderStatusDto, all: readonly ProviderStatusDto[]): string {
  const name = providerName(s.provider);
  const lastOk = s.lastOkMs == null ? "never answered" : `last answered ${formatAsOf(s.lastOkMs)}`;
  switch (s.state) {
    case "HEALTHY":
      return `${name} is answering (${lastOk}).`;
    case "DEGRADED":
      return `${name}: ${s.consecutiveFailures} recent failure(s)${s.lastError ? `, ${s.lastError}` : ""}. Still being tried.`;
    case "DOWN": {
      const by = standIn(s, all);
      const why = s.blocked
        ? `${name} is refusing requests from this network`
        : `${name} is not answering${s.lastError ? ` (${s.lastError})` : ""}`;
      const instead = by ? `${by} is used instead` : "other sources are used";
      return `${why}; ${instead} until it recovers (re-checked at growing intervals). ${lastOk}.`;
    }
  }
}

/** One badge per data provider: `NSE OK`, `YAHOO DOWN`. For the status bar. */
export const ProviderHealth: Component<{ health: HealthState }> = (props) => (
  <output class="health" aria-label="Data provider health">
    <For each={props.health.providers}>
      {(s) => (
        <Badge tone={TONE[s.state]} title={titleFor(s, props.health.providers)}>
          {healthLabel(s, props.health.providers)}
        </Badge>
      )}
    </For>
  </output>
);
