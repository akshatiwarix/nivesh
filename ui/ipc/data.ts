// Typed wrappers for the market-data commands. Components never call `invoke`
// or construct a `Channel` themselves.
import { Channel } from "@tauri-apps/api/core";
import type { HubUpdateDto } from "./bindings/HubUpdateDto";
import type { IndexSnapshotDto } from "./bindings/IndexSnapshotDto";
import type { InstrumentDto } from "./bindings/InstrumentDto";
import type { ProviderStatusDto } from "./bindings/ProviderStatusDto";
import type { SourcedIndexQuoteDto } from "./bindings/SourcedIndexQuoteDto";
import type { SourcedQuoteDto } from "./bindings/SourcedQuoteDto";
import type { TopicDto } from "./bindings/TopicDto";
import { call } from "./core";

export function searchInstruments(query: string, limit?: number): Promise<InstrumentDto[]> {
  return call<InstrumentDto[]>("search_instruments", { query, limit });
}

export function getQuote(exchange: string, symbol: string): Promise<SourcedQuoteDto> {
  return call<SourcedQuoteDto>("get_quote", { exchange, symbol });
}

export function indexSnapshot(index: string): Promise<IndexSnapshotDto> {
  return call<IndexSnapshotDto>("index_snapshot", { index });
}

export function allIndices(): Promise<SourcedIndexQuoteDto[]> {
  return call<SourcedIndexQuoteDto[]>("all_indices");
}

export function providerStatus(): Promise<ProviderStatusDto[]> {
  return call<ProviderStatusDto[]>("provider_status");
}

/** Tells the backend whether the window is visible, so it can pause polling. */
export function setWindowVisible(visible: boolean): Promise<void> {
  return call<void>("set_window_visible", { visible });
}

/**
 * Streams updates for `topics` to `onUpdate` until the returned function is
 * called. The backend polls only topics somebody is subscribed to, so every
 * subscriber must unsubscribe when its view goes away.
 */
export async function subscribeQuotes(
  topics: readonly TopicDto[],
  onUpdate: (update: HubUpdateDto) => void,
): Promise<() => void> {
  const channel = new Channel<HubUpdateDto>();
  channel.onmessage = onUpdate;
  const id = await call<number>("subscribe_quotes", { topics, onUpdate: channel });
  let done = false;
  return () => {
    if (done) return;
    done = true;
    // Fire and forget: a failure here means the subscription is already gone.
    void call<void>("unsubscribe", { id }).catch(() => undefined);
  };
}
