import type { Parsed } from "../commands";
import type { Workspace } from "./workspace";

/**
 * Runs a parsed command against the workspace. Returns an error message
 * for the command bar, or undefined on success.
 */
export function dispatch(parsed: Parsed, ws: Workspace): string | undefined {
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
      return runCommand(parsed.id, ws);
  }
}

/** Runs a registered command by id (also used by function keys). */
export function runCommand(id: string, ws: Workspace): string | undefined {
  switch (id) {
    case "help":
      // Opening HELP twice shouldn't stack two help views.
      if (ws.currentView(ws.state.focused).kind !== "help") ws.open({ kind: "help" });
      return undefined;
    default:
      return `${id.toUpperCase()} is not available yet.`;
  }
}
