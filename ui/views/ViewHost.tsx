import type { Component, JSX } from "solid-js";
import { HelpView } from "./help/HelpView";
import { EmptyView, IndexView, QuoteView } from "./PlaceholderViews";
import type { View } from "./types";
import "./views.css";

function renderView(view: View): JSX.Element {
  switch (view.kind) {
    case "empty":
      return <EmptyView />;
    case "quote":
      return <QuoteView view={view} />;
    case "index":
      return <IndexView view={view} />;
    case "help":
      return <HelpView />;
  }
}

/** Renders the view at the top of a panel's stack. */
export const ViewHost: Component<{ view: View }> = (props) => <>{renderView(props.view)}</>;
