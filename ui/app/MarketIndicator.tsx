import { type Component, createSignal, onCleanup, onMount, Show } from "solid-js";
import { Badge, type BadgeTone } from "../components/Badge";
import {
  inTauri,
  type MarketStatusDto,
  marketStatus,
  onMarketSession,
  type SessionKind,
} from "../ipc";
import { formatIstHm } from "../lib/time";

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

/**
 * NSE session indicator for the header. Reads the status once, then follows the
 * backend's `market:session` event: no polling. Renders nothing outside the app.
 */
export const MarketIndicator: Component = () => {
  const [status, setStatus] = createSignal<MarketStatusDto>();

  onMount(() => {
    if (!inTauri()) return;
    let closed = false;
    let unlisten: (() => void) | undefined;
    // Listen before fetching so a session change in between isn't lost.
    onMarketSession((s) => setStatus(s)).then(
      (stop) => {
        if (closed) stop();
        else unlisten = stop;
      },
      () => undefined,
    );
    marketStatus().then(
      (s) => {
        // An event that already arrived is newer than this snapshot.
        if (!closed && status() === undefined) setStatus(s);
      },
      () => setStatus(undefined),
    );
    onCleanup(() => {
      closed = true;
      unlisten?.();
    });
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
