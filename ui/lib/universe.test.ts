import { describe, expect, it } from "vitest";
import { DEFAULT_UNIVERSE, findUniverse, stepUniverse, UNIVERSES } from "./universe";

describe("universes", () => {
  it("lists every index with constituents, not SENSEX", () => {
    expect(UNIVERSES.map((u) => u.id)).toContain("NIFTYIT");
    expect(UNIVERSES.map((u) => u.id)).not.toContain("SENSEX");
    expect(DEFAULT_UNIVERSE.id).toBe("NIFTY50");
  });

  it("resolves aliases", () => {
    expect(findUniverse("banknifty")?.id).toBe("BANKNIFTY");
    expect(findUniverse("CNXIT")?.id).toBe("NIFTYIT");
    expect(findUniverse("SENSEX")).toBeUndefined();
    expect(findUniverse("RELIANCE")).toBeUndefined();
  });

  it("steps and wraps", () => {
    expect(stepUniverse("NIFTY50", 1).id).toBe(UNIVERSES[1]?.id);
    expect(stepUniverse("NIFTY50", -1).id).toBe(UNIVERSES.at(-1)?.id);
  });
});
