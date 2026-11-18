// The command registry is the single source for dispatch, HELP and
// autocomplete. Adding a command here makes it parse, appear in HELP and
// complete in the command bar.

export type Exchange = "NSE" | "BSE";
export const EXCHANGES: readonly Exchange[] = ["NSE", "BSE"];
export const DEFAULT_EXCHANGE: Exchange = "NSE";

/** A command that takes no instrument: `HELP`, `MKT`, `WL ADD TCS`. */
export interface CommandDef {
  /** Stable identifier used by dispatch. */
  id: string;
  /** Canonical name first, then aliases. Upper-case. */
  names: readonly string[];
  summary: string;
  usage: string;
  /** Function key (1-10) that runs this command. */
  fkey?: number;
}

/** A function applied to an instrument: `RELIANCE CH 5M`. */
export interface FunctionDef {
  id: string;
  names: readonly string[];
  summary: string;
  usage: string;
}

export interface IndexDef {
  id: string;
  /** Upper-case aliases the user may type. */
  aliases: readonly string[];
  title: string;
}

export const COMMANDS: readonly CommandDef[] = [
  {
    id: "help",
    names: ["HELP", "?"],
    summary: "Commands and keys",
    usage: "HELP",
    fkey: 1,
  },
];

/** Functions valid after an instrument. Empty until charts land in Part 3. */
export const FUNCTIONS: readonly FunctionDef[] = [];

/** Index aliases take precedence over stock symbols of the same name. */
export const INDICES: readonly IndexDef[] = [
  { id: "NIFTY50", aliases: ["NIFTY", "NIFTY50", "NSEI"], title: "NIFTY 50" },
  { id: "SENSEX", aliases: ["SENSEX"], title: "SENSEX" },
  { id: "BANKNIFTY", aliases: ["BANKNIFTY", "NIFTYBANK", "BANKEX"], title: "NIFTY BANK" },
];

export function findCommand(name: string): CommandDef | undefined {
  const n = name.toUpperCase();
  return COMMANDS.find((c) => c.names.includes(n));
}

export function findFunction(name: string): FunctionDef | undefined {
  const n = name.toUpperCase();
  return FUNCTIONS.find((f) => f.names.includes(n));
}

export function findIndex(name: string): IndexDef | undefined {
  const n = name.toUpperCase();
  return INDICES.find((i) => i.aliases.includes(n));
}

export function commandForFunctionKey(n: number): CommandDef | undefined {
  return COMMANDS.find((c) => c.fkey === n);
}

/** Every word that can start an input, for autocomplete. */
export function completionWords(): string[] {
  return [
    ...COMMANDS.flatMap((c) => c.names).filter((n) => /^[A-Z]/.test(n)),
    ...INDICES.flatMap((i) => i.aliases),
  ].sort();
}
