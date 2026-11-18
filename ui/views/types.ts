import type { Exchange } from "../commands";
import type { ChartInterval, ChartRange } from "./chart/model";

/** What a panel is showing. Part 1 views are placeholders. */
export type View =
  | { kind: "empty" }
  | { kind: "quote"; exchange: Exchange; symbol: string }
  | { kind: "index"; id: string; title: string }
  | {
      kind: "chart";
      exchange: Exchange;
      symbol: string;
      interval: ChartInterval;
      /** Omitted means the default range for the interval. */
      range?: ChartRange;
    }
  | { kind: "mkt" }
  | { kind: "wl" }
  | { kind: "help" };

export const EMPTY_VIEW: View = { kind: "empty" };

/** Upper-case title for a panel's title bar. */
export function viewTitle(view: View): string {
  switch (view.kind) {
    case "empty":
      return "Empty";
    case "quote":
      return `${view.symbol} · ${view.exchange}`;
    case "index":
      return view.title;
    case "chart":
      return `${view.symbol} · ${view.exchange} · Chart`;
    case "mkt":
      return "MKT · Indices";
    case "wl":
      return "Watchlists";
    case "help":
      return "Help";
  }
}
