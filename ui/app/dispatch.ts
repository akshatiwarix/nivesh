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
      return `${parsed.id.toUpperCase()} is not available yet.`;
  }
}
