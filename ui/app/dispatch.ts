import type { Parsed } from "../commands";
import {
  type ItemRef,
  parseItemRef,
  type WatchlistStore,
  watchlistStore,
} from "../data/watchlists";
import { IpcFailure } from "../ipc";
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
      ws.open({ kind: "quote", exchange: parsed.exchange, symbol: parsed.symbol });
      return undefined;
    case "index":
      ws.open({ kind: "index", id: parsed.id, title: parsed.title });
      return undefined;
    case "command":
      return runCommand(parsed.id, ws, parsed.args);
  }
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
    default:
      return `${id.toUpperCase()} is not available yet.`;
  }
}

export const WL_USAGE = "Usage: WL [ADD|RM SYMBOL… | NEW|USE|DEL NAME]";

function showWatchlist(ws: Workspace) {
  if (ws.currentView(ws.state.focused).kind !== "wl") ws.open({ kind: "wl" });
}

function describe(err: unknown): string {
  return err instanceof IpcFailure || err instanceof Error ? err.message : String(err);
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
      return describe(err);
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
