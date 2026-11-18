import type { Exchange } from "../commands";

/** What a panel is showing. Part 1 views are placeholders. */
export type View =
  | { kind: "empty" }
  | { kind: "quote"; exchange: Exchange; symbol: string }
  | { kind: "index"; id: string; title: string }
  | { kind: "mkt" }
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
    case "mkt":
      return "MKT · Indices";
    case "help":
      return "Help";
  }
}
