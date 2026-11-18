import { type Component, createMemo, For, type JSX, Show } from "solid-js";
import { ProvenanceBadge, providerName } from "../../components/ProvenanceBadge";
import { createFeed, quoteKey } from "../../data/feed";
import type { QuoteDto } from "../../ipc";
import {
  changeTone,
  formatCount,
  formatMove,
  formatMovePct,
  formatPrice,
  formatRange,
  rangePosition,
} from "../../lib/format";
import type { View } from "../types";
import "./QuoteView.css";

type Stat = { label: string; value: () => JSX.Element };

const statsFor = (q: () => QuoteDto): Stat[] => [
  { label: "Open", value: () => formatPrice(q().open) },
  { label: "High", value: () => formatPrice(q().high) },
  { label: "Low", value: () => formatPrice(q().low) },
  { label: "Prev close", value: () => formatPrice(q().prevClose) },
  { label: "Volume", value: () => formatCount(q().volume) },
  {
    label: "Price band",
    value: () =>
      q().lowerBand == null && q().upperBand == null
        ? "—"
        : formatRange(q().lowerBand, q().upperBand),
  },
];

/** `RELIANCE` / `BSE:RELIANCE`: the quote page. */
export const QuoteView: Component<{ view: Extract<View, { kind: "quote" }> }> = (props) => {
  const topic = () =>
    ({ kind: "quote", exchange: props.view.exchange, symbol: props.view.symbol }) as const;
  const feed = createFeed(() => [topic()]);
  const key = () => quoteKey(props.view.exchange, props.view.symbol);
  const sourced = createMemo(() => feed.state.quotes[key()]);
  const error = () => feed.errorFor(topic());

  return (
    <div class="view quote" data-view="quote">
      <Show
        when={sourced()}
        fallback={
          <>
            <h2 class="view__title">
              {props.view.symbol} <span class="view__dim">{props.view.exchange}</span>
            </h2>
            <p class="view__note" role={error() ? "alert" : undefined}>
              <Show when={feed.available} fallback="Market data is available in the Nivesh app.">
                {error()
                  ? error()?.code === "NOT_FOUND"
                    ? `${props.view.symbol} was not found on ${props.view.exchange}. Check the symbol.`
                    : `Quote unavailable: ${error()?.message}. Retrying.`
                  : "Loading quote…"}
              </Show>
            </p>
          </>
        }
      >
        {(s) => {
          const q = () => s().quote;
          const position = () => rangePosition(q().ltp, q().week52Low, q().week52High);
          return (
            <>
              <header class="quote__head">
                <h2 class="view__title">
                  {q().symbol} <span class="view__dim">{q().exchange}</span>
                </h2>
                <span class="quote__name">{q().name ?? ""}</span>
                <ProvenanceBadge provenance={s().provenance} />
              </header>
              <div class="quote__price">
                <span class="quote__ltp">{formatPrice(q().ltp)}</span>
                <span class={changeTone(q().change) ?? ""}>
                  {formatMove(q().change)} ({formatMovePct(q().changePct)})
                </span>
              </div>
              <dl class="quote__grid">
                <For each={statsFor(q)}>
                  {(stat) => (
                    <div class="quote__stat">
                      <dt>{stat.label}</dt>
                      <dd>{stat.value()}</dd>
                    </div>
                  )}
                </For>
              </dl>
              <div class="quote__range">
                <span class="quote__rangelabel">52-week range</span>
                <span class="quote__rangetext">{formatRange(q().week52Low, q().week52High)}</span>
                <Show when={position() !== undefined}>
                  <div
                    class="quote__bar"
                    role="img"
                    aria-label={`Trading at ${Math.round((position() ?? 0) * 100)}% of the 52-week range`}
                  >
                    <div class="quote__mark" style={{ left: `${(position() ?? 0) * 100}%` }} />
                  </div>
                </Show>
              </div>
              <Show when={error()}>
                {(e) => (
                  <p class="view__error" role="alert">
                    Refresh failed: {e().message}. Showing the last data from{" "}
                    {providerName(s().provenance.provider)}.
                  </p>
                )}
              </Show>
            </>
          );
        }}
      </Show>
    </div>
  );
};
