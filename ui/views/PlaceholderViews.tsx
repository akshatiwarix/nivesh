import type { Component } from "solid-js";
import type { View } from "./types";

type Of<K extends View["kind"]> = Extract<View, { kind: K }>;

/** Placeholder until Part 2 provides real quotes. Shows no price, so no provenance badge. */
export const QuoteView: Component<{ view: Of<"quote"> }> = (props) => (
  <div class="view" data-view="quote">
    <h2 class="view__title">
      {props.view.symbol} <span class="view__dim">{props.view.exchange}</span>
    </h2>
    <p class="view__note">Quote page placeholder. Market data arrives in Part 2.</p>
    <p class="view__note">Esc goes back.</p>
  </div>
);

export const IndexView: Component<{ view: Of<"index"> }> = (props) => (
  <div class="view" data-view="index">
    <h2 class="view__title">{props.view.title}</h2>
    <p class="view__note">Index view placeholder. Constituents arrive in Part 2.</p>
    <p class="view__note">Esc goes back.</p>
  </div>
);

export const EmptyView: Component = () => (
  <div class="view" data-view="empty">
    <p class="view__note">Type a command, e.g. RELIANCE or NIFTY.</p>
  </div>
);
