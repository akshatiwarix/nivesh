import { type Component, createSignal, onCleanup, onMount, Show } from "solid-js";
import { Badge, type BadgeTone } from "../components/Badge";
import { inTauri, type MarketStatusDto, marketStatus, type SessionKind } from "../ipc";
import { formatIstHm } from "../lib/time";

const POLL_MS = 15_000;

const LABEL: Record<SessionKind, string> = {
  PRE_OPEN: "PRE-OPEN",
  NORMAL: "MARKET OPEN",
  CLOSING: "CLOSING SESSION",
  CLOSED: "MARKET CLOSED",
};

const TONE: Record<SessionKind, BadgeTone> = {
  PRE_OPEN: "accent",
  NORMAL: "up",
  CLOSING: "accent",
  CLOSED: "neutral",
};

/** Text for the indicator: the session, or the holiday name when closed for one. */
export function statusLabel(s: MarketStatusDto): string {
  return s.holiday && s.session === "CLOSED"
    ? `HOLIDAY: ${s.holiday.toUpperCase()}`
    : LABEL[s.session];
}

/** Tooltip: when the session next changes, in IST. */
export function statusTitle(s: MarketStatusDto): string {
  const next = `Next change ${formatIstHm(s.nextChangeUtcMs)} IST`;
  return s.calendarKnown ? next : `${next}. No holiday data for this year.`;
}

/** NSE session indicator for the header. Renders nothing outside the app. */
export const MarketIndicator: Component = () => {
  const [status, setStatus] = createSignal<MarketStatusDto>();

  const refresh = () => {
    marketStatus().then(setStatus, () => setStatus(undefined));
  };
  onMount(() => {
    if (!inTauri()) return;
    refresh();
    const id = setInterval(refresh, POLL_MS);
    onCleanup(() => clearInterval(id));
  });

  return (
    <Show when={status()}>
      {(s) => (
        <>
          <Badge tone={TONE[s().session]} title={statusTitle(s())}>
            {statusLabel(s())}
          </Badge>
          <Show when={!s().calendarKnown}>
            <Badge tone="stale" title="No holiday data for this year">
              CALENDAR?
            </Badge>
          </Show>
        </>
      )}
    </Show>
  );
};
