import { describe, expect, it } from "vitest";
import type { MarketStatusDto } from "../ipc";
import { statusLabel, statusTitle } from "./MarketIndicator";

const base: MarketStatusDto = {
  session: "NORMAL",
  holiday: null,
  nextChangeUtcMs: Date.UTC(2026, 10, 18, 10, 0), // 15:30 IST
  calendarKnown: true,
};

describe("market indicator text", () => {
  it.each([
    [{}, "MARKET OPEN"],
    [{ session: "PRE_OPEN" }, "PRE-OPEN"],
    [{ session: "CLOSING" }, "CLOSING SESSION"],
    [{ session: "CLOSED" }, "MARKET CLOSED"],
    [{ session: "CLOSED", holiday: "Diwali" }, "HOLIDAY: DIWALI"],
  ] as const)("%j", (over, label) => {
    expect(statusLabel({ ...base, ...over })).toBe(label);
  });

  it("shows the next change in IST and warns about a missing calendar", () => {
    expect(statusTitle(base)).toBe("Next change 15:30 IST");
    expect(statusTitle({ ...base, calendarKnown: false })).toContain("No holiday data");
  });
});
