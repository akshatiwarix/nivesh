import type { Component, JSX } from "solid-js";
import { BreadthView } from "./breadth/BreadthView";
import { ChartView } from "./chart/ChartView";
import { PanelContext } from "./context";
import { HelpView } from "./help/HelpView";
import { IndexView } from "./index/IndexView";
import { MktView } from "./mkt/MktView";
import { MoversView } from "./movers/MoversView";
import { EmptyView } from "./PlaceholderViews";
import { QuoteView } from "./quote/QuoteView";
import { SectorsView } from "./sectors/SectorsView";
import type { View } from "./types";
import { WatchlistView } from "./watchlist/WatchlistView";
import "./views.css";

function renderView(view: View): JSX.Element {
  switch (view.kind) {
    case "empty":
      return <EmptyView />;
    case "quote":
      return <QuoteView view={view} />;
    case "index":
      return <IndexView view={view} />;
    case "chart":
      return <ChartView view={view} />;
    case "mkt":
      return <MktView />;
    case "mov":
      return <MoversView view={view} />;
    case "brd":
      return <BreadthView view={view} />;
    case "sec":
      return <SectorsView />;
    case "wl":
      return <WatchlistView />;
    case "help":
      return <HelpView />;
  }
}

export interface ViewHostProps {
  view: View;
  /** 0-based index of the panel hosting the view. */
  panel: number;
  focused: boolean;
  /** Opens a view in this panel. */
  open: (view: View) => void;
  /** Opens a view in another panel (see `PanelContextValue.openBeside`). */
  openBeside: (view: View) => void;
}

/** Renders the view at the top of a panel's stack, with the panel's context. */
export const ViewHost: Component<ViewHostProps> = (props) => (
  <PanelContext.Provider
    value={{
      get panel() {
        return props.panel;
      },
      open: (view) => props.open(view),
      openBeside: (view) => props.openBeside(view),
      focused: () => props.focused,
    }}
  >
    {renderView(props.view)}
  </PanelContext.Provider>
);
