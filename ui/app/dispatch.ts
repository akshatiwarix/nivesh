import type { Exchange, Parsed } from "../commands";
import { describeError } from "../data/errors";
import {
  type ItemRef,
  parseItemRef,
  type WatchlistStore,
  watchlistStore,
} from "../data/watchlists";
import { parseChartArgs } from "../views/chart/model";
import type { Workspace } from "./workspace";

/** What a command returns: an error message for the command bar, or nothing. */
export type CommandResult = string | undefined | Promise<string | undefined>;

/**
 * Runs a parsed command against the workspace. Returns an error message
 * for the command bar (possibly later, for commands that call the backend),
 * or undefined on success.
 */
export function dispatch(parsed: Parsed, ws: Workspace): CommandResult {
  switch (parsed.kind) {
    case "empty":
      return undefined;
    case "error":
      return parsed.message;
    case "instrument":
      return openInstrument(parsed.exchange, parsed.symbol, parsed.fn, parsed.args, ws);
    case "index":
      ws.open({ kind: "index", id: parsed.id, title: parsed.title });
      return undefined;
    case "command":
      return runCommand(parsed.id, ws, parsed.args);
  }
}

function openInstrument(
  exchange: Exchange,
  symbol: string,
  fn: string | undefined,
  args: readonly string[],
  ws: Workspace,
): CommandResult {
  switch (fn) {
    case undefined:
      ws.open({ kind: "quote", exchange, symbol });
      return undefined;
    case "ch": {
      const chart = parseChartArgs(args);
      if (typeof chart === "string") return chart;
      ws.open({ kind: "chart", exchange, symbol, ...chart });
      return undefined;
    }
    default:
      return `${fn.toUpperCase()} is not available yet.`;
  }
}

/** The instrument the focused panel is showing, if it is showing one. */
export function focusedInstrument(
  ws: Workspace,
): { exchange: Exchange; symbol: string } | undefined {
  const view = ws.currentView(ws.state.focused);
  return view.kind === "quote" || view.kind === "chart"
    ? { exchange: view.exchange, symbol: view.symbol }
    : undefined;
}

/** Runs a registered command by id (also used by function keys). */
export function runCommand(
  id: string,
  ws: Workspace,
  args: readonly string[] = [],
  watchlists: WatchlistStore = watchlistStore,
): CommandResult {
  switch (id) {
    case "help":
      // Opening HELP twice shouldn't stack two help views.
      if (ws.currentView(ws.state.focused).kind !== "help") ws.open({ kind: "help" });
      return undefined;
    case "mkt":
      if (ws.currentView(ws.state.focused).kind !== "mkt") ws.open({ kind: "mkt" });
      return undefined;
    case "wl":
      return runWatchlist(ws, args, watchlists);
    case "ch": {
      const target = focusedInstrument(ws);
      if (!target) return "Open an instrument first, e.g. RELIANCE CH 5M";
      const chart = parseChartArgs(args);
      if (typeof chart === "string") return chart;
      ws.open({ kind: "chart", ...target, ...chart });
      return undefined;
    }
    default:
      return `${id.toUpperCase()} is not available yet.`;
  }
}

export const WL_USAGE = "Usage: WL [ADD|RM SYMBOL… | NEW|USE|DEL NAME]";

function showWatchlist(ws: Workspace) {
  if (ws.currentView(ws.state.focused).kind !== "wl") ws.open({ kind: "wl" });
}

/** `WL`, `WL ADD TCS INFY`, `WL RM TCS`, `WL NEW TECH`, `WL USE TECH`, `WL DEL TECH`. */
function runWatchlist(
  ws: Workspace,
  args: readonly string[],
  store: WatchlistStore,
): CommandResult {
  const [sub, ...rest] = args;
  if (sub === undefined) {
    showWatchlist(ws);
    return undefined;
  }
  const refs: ItemRef[] = rest.map(parseItemRef);
  const name = rest.join(" ");
  const act = async (work: () => Promise<void>): Promise<string | undefined> => {
    try {
      await store.load();
      await work();
      showWatchlist(ws);
      return undefined;
    } catch (err) {
      return describeError(err);
    }
  };

  switch (sub) {
    case "ADD":
      return rest.length === 0
        ? "WL ADD needs a symbol, e.g. WL ADD TCS"
        : act(() => store.add(refs));
    case "RM":
    case "REMOVE":
      return rest.length === 0
        ? "WL RM needs a symbol, e.g. WL RM TCS"
        : act(() => store.remove(refs));
    case "NEW":
      return name === "" ? "WL NEW needs a name, e.g. WL NEW TECH" : act(() => store.create(name));
    case "USE":
      return name === ""
        ? "WL USE needs a list name or number"
        : act(async () => {
            const list = store.findList(name);
            if (!list) throw new Error(`No watchlist named ${name}`);
            await store.activate(list.id);
          });
    case "DEL":
      return name === ""
        ? "WL DEL needs the list's name, e.g. WL DEL TECH"
        : act(async () => {
            // Deleting is destructive: the exact name must be typed.
            const list = store.state.lists.find((l) => l.name === name.toUpperCase());
            if (!list) throw new Error(`No watchlist named ${name}`);
            await store.delete(list.id);
          });
    default:
      return WL_USAGE;
  }
}
