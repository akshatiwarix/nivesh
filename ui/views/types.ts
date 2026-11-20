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
  | { kind: "mov"; index: string }
  | { kind: "brd"; index: string }
  | { kind: "sec" }
  | { kind: "news"; symbol?: string }
  | { kind: "wl" }
  | { kind: "port" }
  | { kind: "pos" }
  | { kind: "ord" }
  | { kind: "trd" }
  | { kind: "funds" }
  | { kind: "help" }
  | { kind: "strat" }
  | { kind: "bt"; name?: string }
  | { kind: "paper" }
  | { kind: "risk" };

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
    case "mov":
      return "Movers";
    case "brd":
      return "Breadth";
    case "sec":
      return "Sectors";
    case "news":
      return view.symbol ? `${view.symbol} · News` : "News";
    case "mkt":
      return "MKT · Indices";
    case "wl":
      return "Watchlists";
    case "port":
      return "Portfolio";
    case "pos":
      return "Positions";
    case "ord":
      return "Orders";
    case "trd":
      return "Trades";
    case "funds":
      return "Funds";
    case "strat":
      return "Strategies";
    case "bt":
      return "Backtest";
    case "paper":
      return "Paper Trading";
    case "risk":
      return "Risk";
    case "help":
      return "Help";
  }
}
