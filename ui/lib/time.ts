// Display-only time helpers. Nivesh stores time in UTC and shows it in IST,
// which is a fixed UTC+05:30 with no daylight saving, so no tz database is needed.

export const IST_OFFSET_MINUTES = 330;

const WEEKDAYS = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"] as const;
const MONTHS = [
  "JAN",
  "FEB",
  "MAR",
  "APR",
  "MAY",
  "JUN",
  "JUL",
  "AUG",
  "SEP",
  "OCT",
  "NOV",
  "DEC",
] as const;

const pad2 = (n: number) => String(n).padStart(2, "0");

/** A Date whose UTC fields read as IST wall-clock fields. */
function istFields(at: Date | number) {
  const ms = typeof at === "number" ? at : at.getTime();
  const d = new Date(ms + IST_OFFSET_MINUTES * 60_000);
  return {
    year: d.getUTCFullYear(),
    month: d.getUTCMonth(),
    day: d.getUTCDate(),
    weekday: d.getUTCDay(),
    hours: d.getUTCHours(),
    minutes: d.getUTCMinutes(),
    seconds: d.getUTCSeconds(),
  };
}

/** `15:42:07` */
export function formatIstTime(at: Date | number): string {
  const f = istFields(at);
  return `${pad2(f.hours)}:${pad2(f.minutes)}:${pad2(f.seconds)}`;
}

/** `15:42` */
export function formatIstHm(at: Date | number): string {
  const f = istFields(at);
  return `${pad2(f.hours)}:${pad2(f.minutes)}`;
}

/** `WED 18 NOV 2026` */
export function formatIstDate(at: Date | number): string {
  const f = istFields(at);
  return `${WEEKDAYS[f.weekday]} ${pad2(f.day)} ${MONTHS[f.month]} ${f.year}`;
}

const IST_DAY_MS = 86_400_000;

function istDayNumber(ms: number): number {
  return Math.floor((ms + IST_OFFSET_MINUTES * 60_000) / IST_DAY_MS);
}

/**
 * An as-of time for a data label: `15:30:00 IST` when it is from today (IST),
 * `17 NOV 15:30 IST` otherwise, so old data never looks current.
 */
export function formatAsOf(asOfMs: number, nowMs: number = Date.now()): string {
  if (istDayNumber(asOfMs) === istDayNumber(nowMs)) return `${formatIstTime(asOfMs)} IST`;
  const f = istFields(asOfMs);
  return `${pad2(f.day)} ${MONTHS[f.month]} ${formatIstHm(asOfMs)} IST`;
}

/** `12s`, `5m`, `2h`, `3d`: how long ago, coarsely. */
export function formatAge(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86_400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86_400)}d`;
}
