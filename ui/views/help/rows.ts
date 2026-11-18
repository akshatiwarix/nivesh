import { COMMANDS, INDICES } from "../../commands";
import { type Binding, KEYMAP } from "../../keys/keymap";

export interface HelpRow {
  keys: string;
  summary: string;
}

/** Every registered command, built from the registry so HELP can't drift. */
export function commandRows(): HelpRow[] {
  return COMMANDS.map((c) => ({
    keys: [c.usage, ...(c.fkey ? [`F${c.fkey}`] : [])].join("  "),
    summary: c.summary,
  }));
}

/** How to address instruments and indices. */
export function targetRows(): HelpRow[] {
  return [
    { keys: "SYMBOL", summary: "NSE quote, e.g. RELIANCE" },
    { keys: "EXCH:SYMBOL", summary: "Quote on NSE or BSE, e.g. BSE:RELIANCE" },
    ...INDICES.map((i) => ({ keys: i.aliases.join(" / "), summary: `${i.title} index` })),
  ];
}

/** Every binding in the keymap. F-keys without a command are left out. */
export function keyRows(keymap: readonly Binding[] = KEYMAP): HelpRow[] {
  const assigned = new Set(COMMANDS.flatMap((c) => (c.fkey ? [c.fkey] : [])));
  return keymap
    .filter((b) => b.action.type !== "fkey" || assigned.has(b.action.n))
    .map((b) => {
      const label =
        b.action.type === "fkey"
          ? (COMMANDS.find((c) => c.fkey === (b.action as { n: number }).n)?.summary ?? b.summary)
          : b.summary;
      return { keys: b.keys.join(" / "), summary: label };
    });
}
