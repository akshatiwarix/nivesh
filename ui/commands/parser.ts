// Pure command grammar: `[EXCH:]TARGET [FUNCTION] [ARGS...]`, case-insensitive.
// No DOM, no IPC: the output feeds dispatch, so it is easy to test.

import {
  DEFAULT_EXCHANGE,
  EXCHANGES,
  type Exchange,
  findCommand,
  findFunction,
  findIndex,
} from "./registry";

export type Parsed =
  | { kind: "empty" }
  | { kind: "command"; id: string; args: string[] }
  | { kind: "index"; id: string; title: string; fn?: string; args: string[] }
  | { kind: "instrument"; exchange: Exchange; symbol: string; fn?: string; args: string[] }
  | { kind: "error"; message: string };

// Mirrors nivesh-core's InstrumentId: first char alphanumeric, then
// alphanumerics and `& - _ .`.
const SYMBOL = /^[A-Z0-9][A-Z0-9&\-_.]*$/;
const MAX_SYMBOL = 24;

export function parse(input: string): Parsed {
  const tokens = input.trim().toUpperCase().split(/\s+/).filter(Boolean);
  const head = tokens[0];
  if (head === undefined) return { kind: "empty" };
  const rest = tokens.slice(1);

  const colon = head.indexOf(":");
  if (colon >= 0) {
    const exch = head.slice(0, colon);
    const symbol = head.slice(colon + 1);
    if (!isExchange(exch)) {
      return { kind: "error", message: `Unknown exchange "${exch}". Use NSE or BSE.` };
    }
    if (symbol === "") return { kind: "error", message: `Missing symbol after "${exch}:".` };
    return instrument(exch, symbol, rest);
  }

  const command = findCommand(head);
  if (command) return { kind: "command", id: command.id, args: rest };

  const index = findIndex(head);
  if (index) {
    const fn = withFunction(rest);
    if (fn.kind === "error") return fn;
    return { kind: "index", id: index.id, title: index.title, ...fn.value };
  }

  return instrument(DEFAULT_EXCHANGE, head, rest);
}

function instrument(exchange: Exchange, symbol: string, rest: string[]): Parsed {
  if (symbol.length > MAX_SYMBOL || !SYMBOL.test(symbol)) {
    return { kind: "error", message: `Invalid symbol "${symbol}".` };
  }
  const fn = withFunction(rest);
  if (fn.kind === "error") return fn;
  return { kind: "instrument", exchange, symbol, ...fn.value };
}

function withFunction(
  rest: string[],
): { kind: "ok"; value: { fn?: string; args: string[] } } | { kind: "error"; message: string } {
  const [name, ...args] = rest;
  if (name === undefined) return { kind: "ok", value: { args: [] } };
  const def = findFunction(name);
  if (!def) return { kind: "error", message: `Unknown function "${name}".` };
  return { kind: "ok", value: { fn: def.id, args } };
}

function isExchange(value: string): value is Exchange {
  return (EXCHANGES as readonly string[]).includes(value);
}
