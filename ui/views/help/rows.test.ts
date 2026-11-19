import { describe, expect, it } from "vitest";
import { COMMANDS, INDICES } from "../../commands";
import { KEYMAP } from "../../keys/keymap";
import { commandRows, keyRows, targetRows } from "./rows";

describe("help rows", () => {
  it("lists every registered command with its function key", () => {
    const rows = commandRows();
    expect(rows).toHaveLength(COMMANDS.length);
    expect(rows.find((r) => r.keys.includes("HELP"))?.keys).toContain("F1");
  });

  it("lists every index alias", () => {
    const text = targetRows()
      .map((r) => r.keys)
      .join(" ");
    for (const alias of INDICES.flatMap((i) => i.aliases)) expect(text).toContain(alias);
  });

  it("lists every non-fkey binding and only assigned F-keys", () => {
    const rows = keyRows();
    const nonF = KEYMAP.filter((b) => b.action.type !== "fkey");
    for (const b of nonF) expect(rows.some((r) => r.keys === b.keys.join(" / "))).toBe(true);
    // F1-F10 are all assigned now (F10 = STRAT); F11/F12 stay unassigned
    // because macOS reserves them.
    for (const n of ["F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10"]) {
      expect(rows.some((r) => r.keys.startsWith(`${n} `))).toBe(true);
    }
    expect(rows.some((r) => r.keys.startsWith("F11"))).toBe(false);
    expect(rows.some((r) => r.keys.startsWith("F12"))).toBe(false);
  });
});
