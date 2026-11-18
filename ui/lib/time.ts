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
