// Decides what, if anything, the command bar should search instruments for,
// and what line accepting a suggestion runs. Pure.

import type { InstrumentDto } from "../ipc";
import { findCommand } from "./registry";

export const MAX_QUERY = 64;

export interface SearchTarget {
  /** Text to search the instrument list for. */
  query: string;
  /** The command line that accepting `instrument` runs. */
  line: (instrument: InstrumentDto) => string;
}

/** `NSE:RELIANCE`. The explicit exchange keeps it a stock, never an index alias. */
export function instrumentLine(i: Pick<InstrumentDto, "exchange" | "symbol">): string {
  return `${i.exchange}:${i.symbol}`;
}

/**
 * What to search for given the text in the bar. Commands and `EXCH:` prefixed
 * input are left alone: once you type a command you want the command.
 */
export function searchTarget(text: string): SearchTarget | undefined {
  const trimmed = text.trim();
  if (trimmed === "" || trimmed.length > MAX_QUERY) return undefined;
  const first = trimmed.split(/\s+/)[0] ?? "";
  if (first.includes(":") || findCommand(first)) return undefined;
  return { query: trimmed, line: instrumentLine };
}
