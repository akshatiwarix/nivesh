import { describe, expect, it } from "vitest";
import { KEYMAP, type KeyLike, resolveKey } from "./keymap";

const ev = (over: Partial<KeyLike>): KeyLike => ({
  key: "",
  code: "",
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...over,
});

describe("resolveKey", () => {
  it.each([
    [
      { key: "F1", code: "F1" },
      { type: "fkey", n: 1 },
    ],
    [
      { key: "F10", code: "F10" },
      { type: "fkey", n: 10 },
    ],
    [
      { key: "3", code: "Digit3", ctrlKey: true },
      { type: "fkey", n: 3 },
    ],
    [
      { key: "0", code: "Digit0", ctrlKey: true },
      { type: "fkey", n: 10 },
    ],
    [
      { key: "Tab", code: "Tab" },
      { type: "focus", delta: 1 },
    ],
    [
      { key: "Tab", code: "Tab", shiftKey: true },
      { type: "focus", delta: -1 },
    ],
    // On macOS Option+4 produces "¢": the physical code must still match.
    [
      { key: "¢", code: "Digit4", altKey: true },
      { type: "layout", layout: 4 },
    ],
    [{ key: "?", code: "Slash", shiftKey: true }, { type: "help" }],
    [{ key: "Escape", code: "Escape" }, { type: "back" }],
  ])("%j", (e, action) => {
    expect(resolveKey(ev(e), false)?.action).toEqual(action);
  });

  it("ignores unbound keys and extra modifiers", () => {
    expect(resolveKey(ev({ key: "a", code: "KeyA" }), false)).toBeUndefined();
    expect(resolveKey(ev({ key: "F1", code: "F1", ctrlKey: true }), false)).toBeUndefined();
    expect(resolveKey(ev({ key: "F1", code: "F1", metaKey: true }), false)).toBeUndefined();
    expect(resolveKey(ev({ key: "1", code: "Digit1" }), false)).toBeUndefined();
  });

  it("does not hijack typing keys while an input is focused", () => {
    expect(resolveKey(ev({ key: "?", code: "Slash", shiftKey: true }), true)).toBeUndefined();
    expect(resolveKey(ev({ key: "Escape", code: "Escape" }), true)).toBeUndefined();
    // Chords that can't produce text still work from the command bar.
    expect(resolveKey(ev({ key: "F1", code: "F1" }), true)?.action.type).toBe("fkey");
    expect(resolveKey(ev({ key: "2", code: "Digit2", altKey: true }), true)?.action.type).toBe(
      "layout",
    );
  });

  it("has unique binding ids and no chord bound twice", () => {
    const ids = KEYMAP.map((b) => b.id);
    expect(new Set(ids).size).toBe(ids.length);
    const names = KEYMAP.flatMap((b) => b.keys);
    expect(new Set(names).size).toBe(names.length);
  });
});
