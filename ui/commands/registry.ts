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
  {
    id: "wl",
    names: ["WL", "WATCHLIST"],
    summary: "Watchlists. WL ADD|RM <SYM>, WL NEW|USE|DEL <NAME>",
    usage: "WL",
    fkey: 3,
  },
  {
    id: "mkt",
    names: ["MKT", "MARKETS"],
    summary: "Index dashboard: NIFTY 50, SENSEX, NIFTY BANK and sector indices",
    usage: "MKT",
    fkey: 2,
  },
  {
    id: "ch",
    names: ["CH", "CHART"],
    summary: "Chart of the instrument in the focused panel. CH 5M, CH 1D 5Y",
    usage: "CH [INTERVAL] [RANGE]",
    fkey: 4,
  },
  {
    id: "mov",
    names: ["MOV", "MOVERS"],
    summary: "Top gainers, losers and most active. MOV BANKNIFTY picks the index",
    usage: "MOV [INDEX]",
    fkey: 5,
  },
  {
    id: "sec",
    names: ["SEC", "SECTORS"],
    summary: "Sector returns (1D 1W 1M YTD) and a heatmap of the selected index's stocks",
    usage: "SEC",
    fkey: 6,
  },
  {
    id: "brd",
    names: ["BRD", "BREADTH"],
    summary: "Advances/declines, and % of NIFTY 50/100 above their 20/50/200-day averages",
    usage: "BRD [INDEX]",
  },
  {
    id: "add",
    names: ["ADD"],
    summary:
      "Add an indicator to the focused chart: SMA EMA WMA BOLL VWAP (on price), RSI MACD ATR (own pane)",
    usage: "ADD <INDICATOR> [PERIOD…], e.g. ADD SMA 50, ADD RSI 14",
  },
  {
    id: "rm",
    names: ["RM", "REMOVE"],
    summary: "Remove an indicator from the focused chart: RM RSI, or RM SMA 50 for one of several",
    usage: "RM <INDICATOR> [PERIOD…]",
  },
];

/** Functions valid after an instrument: `RELIANCE CH 5M`. */
export const FUNCTIONS: readonly FunctionDef[] = [
  {
    id: "ch",
    names: ["CH", "CHART"],
    summary:
      "Candlestick chart. INTERVAL: 1M 5M 15M 30M 1H 4H 1D 1W 1MO. RANGE: 1D 5D 1MO 3MO 6MO 1Y 5Y MAX",
    usage: "<SYMBOL> CH [INTERVAL] [RANGE]",
  },
];

/**
 * Index aliases take precedence over stock symbols of the same name.
 * Mirrors the catalogue in `nivesh-core` (ids are its codes); a Rust test
 * checks the two stay in sync.
 */
export const INDICES: readonly IndexDef[] = [
  { id: "NIFTY50", aliases: ["NIFTY", "NIFTY50", "NSEI"], title: "NIFTY 50" },
  { id: "SENSEX", aliases: ["SENSEX", "BSESN"], title: "SENSEX" },
  { id: "BANKNIFTY", aliases: ["BANKNIFTY", "NIFTYBANK", "NSEBANK"], title: "NIFTY BANK" },
  { id: "NIFTYIT", aliases: ["NIFTYIT", "CNXIT"], title: "NIFTY IT" },
  { id: "NIFTYAUTO", aliases: ["NIFTYAUTO", "CNXAUTO"], title: "NIFTY AUTO" },
  { id: "NIFTYPHARMA", aliases: ["NIFTYPHARMA", "CNXPHARMA"], title: "NIFTY PHARMA" },
  { id: "NIFTYFMCG", aliases: ["NIFTYFMCG", "CNXFMCG"], title: "NIFTY FMCG" },
  { id: "NIFTYMETAL", aliases: ["NIFTYMETAL", "CNXMETAL"], title: "NIFTY METAL" },
  { id: "NIFTYREALTY", aliases: ["NIFTYREALTY", "CNXREALTY"], title: "NIFTY REALTY" },
  { id: "NIFTYENERGY", aliases: ["NIFTYENERGY", "CNXENERGY"], title: "NIFTY ENERGY" },
  {
    id: "NIFTYPSUBANK",
    aliases: ["NIFTYPSUBANK", "PSUBANK", "CNXPSUBANK"],
    title: "NIFTY PSU BANK",
  },
  {
    id: "NIFTYFINSERVICE",
    aliases: ["NIFTYFINSERVICE", "FINNIFTY", "NIFTYFIN"],
    title: "NIFTY FIN SERVICE",
  },
  { id: "NIFTYMEDIA", aliases: ["NIFTYMEDIA", "CNXMEDIA"], title: "NIFTY MEDIA" },
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
