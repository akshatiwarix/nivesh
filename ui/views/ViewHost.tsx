import type { Component, JSX } from "solid-js";
import { AuditView } from "./audit/AuditView";
import { BreadthView } from "./breadth/BreadthView";
import {
  FundsView,
  OrdersView,
  PortfolioView,
  PositionsView,
  TradesView,
} from "./broker/BrokerViews";
import { ChartView } from "./chart/ChartView";
import { PanelContext } from "./context";
import { HelpView } from "./help/HelpView";
import { IndexView } from "./index/IndexView";
import { MktView } from "./mkt/MktView";
import { MoversView } from "./movers/MoversView";
import { NewsView } from "./news/NewsView";
import { EmptyView } from "./PlaceholderViews";
import { QuoteView } from "./quote/QuoteView";
import { RiskView } from "./risk/RiskView";
import { ScriptView } from "./script/ScriptView";
import { SectorsView } from "./sectors/SectorsView";
import { SettingsView } from "./settings/SettingsView";
import { BacktestView, PaperDashboardView, StrategiesView } from "./strategy/StrategyViews";
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
    case "news":
      return <NewsView view={view} />;
    case "wl":
      return <WatchlistView />;
    case "port":
      return <PortfolioView />;
    case "pos":
      return <PositionsView />;
    case "ord":
      return <OrdersView />;
    case "trd":
      return <TradesView />;
    case "funds":
      return <FundsView />;
    case "help":
      return <HelpView />;
    case "strat":
      return <StrategiesView />;
    case "bt":
      return <BacktestView name={view.name} />;
    case "paper":
      return <PaperDashboardView />;
    case "risk":
      return <RiskView />;
    case "script":
      return <ScriptView view={view} />;
    case "settings":
      return <SettingsView view={view} />;
    case "audit":
      return <AuditView view={view} />;
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
