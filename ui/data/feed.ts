// The live data feed for one view: subscribes to backend topics, applies the
// updates in animation-frame batches, and exposes them as a fine-grained store
// so a changing price updates exactly its own text node.
import { type Accessor, batch, createEffect, createSignal, onCleanup } from "solid-js";
import { createStore, produce, reconcile, type SetStoreFunction } from "solid-js/store";
import type {
  HubUpdateDto,
  IndexQuoteDto,
  IpcError,
  ProvenanceDto,
  QuoteDto,
  SourcedIndexQuoteDto,
  SourcedQuoteDto,
  TopicDto,
} from "../ipc";
import { inTauri, subscribeQuotes, toIpcError } from "../ipc";

export const quoteKey = (exchange: string, symbol: string) => `${exchange}:${symbol}`;

export function topicKey(t: TopicDto): string {
  switch (t.kind) {
    case "quote":
      return `quote:${quoteKey(t.exchange, t.symbol)}`;
    case "index":
      return `index:${t.index}`;
    case "allIndices":
      return "allIndices";
  }
}

/** A single index: its level and where it came from. */
export interface IndexHeader {
  quote: IndexQuoteDto;
  provenance: ProvenanceDto;
}

export interface FeedState {
  /** Stock quotes by `EXCH:SYMBOL`. */
  quotes: Partial<Record<string, SourcedQuoteDto>>;
  /** Dashboard index rows by index code, with a stable display order. */
  indexRows: Partial<Record<string, SourcedIndexQuoteDto>>;
  indexOrder: string[];
  /** The one index being viewed, with its constituents. */
  header?: IndexHeader;
  constituents: Partial<Record<string, QuoteDto>>;
  constituentOrder: string[];
  /** Latest failure per topic (see {@link topicKey}). Cleared when the topic next delivers. */
  errors: Partial<Record<string, IpcError>>;
}

export function emptyFeedState(): FeedState {
  return {
    quotes: {},
    indexRows: {},
    indexOrder: [],
    constituents: {},
    constituentOrder: [],
    errors: {},
  };
}

function sameOrder(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && a.every((v, i) => v === b[i]);
}

/** Deletes every key of `record` that is not in `keep`. */
function dropOthers(record: Partial<Record<string, unknown>>, keep: readonly string[]) {
  for (const key of Object.keys(record)) {
    if (!keep.includes(key)) delete record[key];
  }
}

/**
 * Applies one backend update to the store. Records are reconciled in place, so
 * object identity is kept and rows are never re-created for a price change.
 */
export function applyUpdate(set: SetStoreFunction<FeedState>, update: HubUpdateDto): void {
  batch(() => {
    switch (update.kind) {
      case "quotes":
        for (const q of update.quotes) {
          const key = quoteKey(q.quote.exchange, q.quote.symbol);
          set("quotes", key, reconcile(q));
          set("errors", `quote:${key}`, undefined);
        }
        return;
      case "allIndices": {
        const order = update.rows.map((r) => r.quote.index);
        for (const r of update.rows) set("indexRows", r.quote.index, reconcile(r));
        set(produce((s) => dropOthers(s.indexRows, order)));
        set("indexOrder", (prev) => (sameOrder(prev, order) ? prev : order));
        set("errors", "allIndices", undefined);
        return;
      }
      case "index": {
        const s = update.snapshot;
        const order = s.constituents.map((c) => c.symbol);
        set("header", reconcile({ quote: s.quote, provenance: s.provenance }));
        for (const c of s.constituents) set("constituents", c.symbol, reconcile(c));
        // Drop constituents that left the index.
        set(produce((s) => dropOthers(s.constituents, order)));
        set("constituentOrder", (prev) => (sameOrder(prev, order) ? prev : order));
        set("errors", `index:${s.quote.index}`, undefined);
        return;
      }
      case "failed":
        set("errors", topicKey(update.topic), update.error);
        return;
    }
  });
}

type Frame = (run: () => void) => void;

const nextFrame: Frame = (run) => {
  if (typeof requestAnimationFrame === "function") requestAnimationFrame(() => run());
  else setTimeout(run, 16);
};

export interface Feed {
  state: FeedState;
  /** False outside the app (browser, tests): there is no backend to ask. */
  available: boolean;
  /** Set if the subscription itself could not be started. */
  fatal: Accessor<IpcError | undefined>;
  /** The latest failure for a topic: the subscription's, or that topic's own. */
  errorFor: (topic: TopicDto) => IpcError | undefined;
}

/**
 * Subscribes to `topics` for as long as the calling component lives (and
 * re-subscribes when the topics change). Updates are applied once per
 * animation frame.
 */
export function createFeed(topics: Accessor<readonly TopicDto[]>, frame: Frame = nextFrame): Feed {
  const [state, setState] = createStore<FeedState>(emptyFeedState());
  const [fatal, setFatal] = createSignal<IpcError>();
  const available = inTauri();

  createEffect(() => {
    const list = topics();
    if (!available || list.length === 0) return;
    let closed = false;
    let stop: (() => void) | undefined;
    let pending: HubUpdateDto[] = [];
    let scheduled = false;

    const flush = () => {
      scheduled = false;
      const updates = pending;
      pending = [];
      if (closed) return;
      for (const u of updates) applyUpdate(setState, u);
    };
    const onUpdate = (u: HubUpdateDto) => {
      pending.push(u);
      if (!scheduled) {
        scheduled = true;
        frame(flush);
      }
    };

    setFatal(undefined);
    subscribeQuotes(list, onUpdate).then(
      (unsubscribe) => {
        // The view may have gone away while the subscription was starting.
        if (closed) unsubscribe();
        else stop = unsubscribe;
      },
      (reason: unknown) => {
        if (!closed) setFatal(toIpcError(reason));
      },
    );
    onCleanup(() => {
      closed = true;
      stop?.();
    });
  });

  const errorFor = (topic: TopicDto) => fatal() ?? state.errors[topicKey(topic)];
  return { state, available, fatal, errorFor };
}
