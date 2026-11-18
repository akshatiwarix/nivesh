import { describe, expect, it } from "vitest";
import { formatIstDate, formatIstHm, formatIstTime } from "./time";

describe("IST formatting", () => {
  it.each([
    // [UTC instant, IST time, IST date]
    ["2026-11-18T04:45:07Z", "10:15:07", "WED 18 NOV 2026"],
    ["2026-11-18T18:29:59Z", "23:59:59", "WED 18 NOV 2026"],
    // 18:30 UTC is midnight IST: the date rolls over.
    ["2026-11-18T18:30:00Z", "00:00:00", "THU 19 NOV 2026"],
    ["2026-12-31T20:00:00Z", "01:30:00", "FRI 01 JAN 2027"],
    ["2026-03-01T00:00:00Z", "05:30:00", "SUN 01 MAR 2026"],
  ])("%s -> %s %s", (utc, time, date) => {
    const at = new Date(utc);
    expect(formatIstTime(at)).toBe(time);
    expect(formatIstDate(at)).toBe(date);
    expect(formatIstHm(at.getTime())).toBe(time.slice(0, 5));
  });
});
