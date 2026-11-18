import { createRoot } from "solid-js";
import { createStore } from "solid-js/store";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { HubUpdateDto } from "../ipc";
import { subscribeQuotes } from "../ipc";
import { applyUpdate, createFeed, emptyFeedState, topicKey } from "./feed";
import { allIndicesUpdate, indexRow, indexUpdate, prov, quote } from "./fixtures";

vi.mock("../ipc", () => ({
  inTauri: () => true,
  subscribeQuotes: vi.fn(),
  toIpcError: (e: unknown) => ({ code: "INTERNAL", message: String(e), retryable: false }),
}));

const subscribe = vi.mocked(subscribeQuotes);

function store() {
  return createStore(emptyFeedState());
}

describe("applyUpdate", () => {
  it("keeps row identity across updates so rows are not re-created", () => {
    const [state, set] = store();
    applyUpdate(set, allIndicesUpdate(indexRow("NIFTY50", "NIFTY 50", 25000)));
    const before = state.indexRows["NIFTY50"];
    applyUpdate(set, allIndicesUpdate(indexRow("NIFTY50", "NIFTY 50", 25010)));
    expect(state.indexRows["NIFTY50"]).toBe(before);
    expect(state.indexRows["NIFTY50"]?.quote.last).toBe(25010);
    expect(state.indexOrder).toEqual(["NIFTY50"]);
  });

  it("follows the order and drops indices that disappear", () => {
    const [state, set] = store();
    applyUpdate(
      set,
      allIndicesUpdate(indexRow("A", "A", 1), indexRow("B", "B", 2), indexRow("C", "C", 3)),
    );
    applyUpdate(set, allIndicesUpdate(indexRow("C", "C", 3), indexRow("A", "A", 1)));
    expect(state.indexOrder).toEqual(["C", "A"]);
    expect(Object.keys(state.indexRows).sort()).toEqual(["A", "C"]);
  });

  it("updates constituents in place and drops ones that left the index", () => {
    const [state, set] = store();
    applyUpdate(set, indexUpdate("NIFTY50", "NIFTY 50", 25000, ["RELIANCE", "TCS", "INFY"]));
    const reliance = state.constituents["RELIANCE"];
    applyUpdate(set, indexUpdate("NIFTY50", "NIFTY 50", 25001, ["RELIANCE", "TCS"]));
    expect(state.constituents["RELIANCE"]).toBe(reliance);
    expect(state.constituentOrder).toEqual(["RELIANCE", "TCS"]);
    expect(state.constituents["INFY"]).toBeUndefined();
    expect(state.header?.quote.last).toBe(25001);
    expect(state.header?.provenance.provider).toBe("nse");
  });

  it("a snapshot with no constituents leaves an empty list under the same header", () => {
    const [state, set] = store();
    applyUpdate(set, indexUpdate("NIFTY50", "NIFTY 50", 25000, ["TCS"]));
    applyUpdate(set, indexUpdate("NIFTY50", "NIFTY 50", 25000, [], "yahoo"));
    expect(state.constituentOrder).toEqual([]);
    expect(state.header?.provenance.provider).toBe("yahoo");
  });

  it("records failures per topic and clears them when data arrives", () => {
    const [state, set] = store();
    const error = { code: "UNAVAILABLE", message: "blocked", retryable: true } as const;
    applyUpdate(set, { kind: "failed", topic: { kind: "allIndices" }, error });
    expect(state.errors["allIndices"]?.message).toBe("blocked");
    applyUpdate(set, allIndicesUpdate(indexRow("A", "A", 1)));
    expect(state.errors["allIndices"]).toBeUndefined();

    applyUpdate(set, {
      kind: "failed",
      topic: { kind: "quote", exchange: "NSE", symbol: "TCS" },
      error,
    });
    expect(state.errors["quote:NSE:TCS"]).toBeDefined();
    applyUpdate(set, {
      kind: "quotes",
      quotes: [{ quote: quote("TCS", 3000), provenance: prov() }],
    });
    expect(state.errors["quote:NSE:TCS"]).toBeUndefined();
    expect(state.quotes["NSE:TCS"]?.quote.ltp).toBe(3000);
  });

  it("topic keys are stable", () => {
    expect(topicKey({ kind: "allIndices" })).toBe("allIndices");
    expect(topicKey({ kind: "index", index: "NIFTY50" })).toBe("index:NIFTY50");
    expect(topicKey({ kind: "quote", exchange: "NSE", symbol: "TCS" })).toBe("quote:NSE:TCS");
  });
});

describe("createFeed", () => {
  let frames: (() => void)[];
  const frame = (run: () => void) => frames.push(run);
  const runFrames = () => {
    const todo = frames;
    frames = [];
    for (const f of todo) f();
  };
  const flushMicrotasks = () => new Promise<void>((r) => setTimeout(r, 0));

  beforeEach(() => {
    frames = [];
    subscribe.mockReset();
  });

  it("applies a burst of updates in one animation frame", async () => {
    let push: (u: HubUpdateDto) => void = () => {};
    subscribe.mockImplementation(async (_t, onUpdate) => {
      push = onUpdate;
      return () => {};
    });
    await createRoot(async (dispose) => {
      const feed = createFeed(() => [{ kind: "allIndices" }], frame);
      await flushMicrotasks();
      push(allIndicesUpdate(indexRow("A", "A", 1)));
      push(allIndicesUpdate(indexRow("A", "A", 2)));
      push(allIndicesUpdate(indexRow("A", "A", 3)));
      expect(frames).toHaveLength(1);
      expect(feed.state.indexOrder).toEqual([]);
      runFrames();
      expect(feed.state.indexRows["A"]?.quote.last).toBe(3);
      dispose();
    });
  });

  it("unsubscribes when its owner is disposed and ignores late updates", async () => {
    const stop = vi.fn();
    let push: (u: HubUpdateDto) => void = () => {};
    subscribe.mockImplementation(async (_t, onUpdate) => {
      push = onUpdate;
      return stop;
    });
    let feed!: ReturnType<typeof createFeed>;
    const dispose = createRoot((d) => {
      feed = createFeed(() => [{ kind: "allIndices" }], frame);
      return d;
    });
    await flushMicrotasks();
    push(allIndicesUpdate(indexRow("A", "A", 1)));
    dispose();
    expect(stop).toHaveBeenCalledTimes(1);
    runFrames();
    expect(feed.state.indexOrder).toEqual([]);
  });

  it("unsubscribes at once if disposed before the subscription was established", async () => {
    const stop = vi.fn();
    let resolve!: (fn: () => void) => void;
    subscribe.mockReturnValue(new Promise((r) => (resolve = r)));
    const dispose = createRoot((d) => {
      createFeed(() => [{ kind: "allIndices" }], frame);
      return d;
    });
    dispose();
    resolve(stop);
    await flushMicrotasks();
    expect(stop).toHaveBeenCalledTimes(1);
  });

  it("does not subscribe to an empty topic list", async () => {
    createRoot((dispose) => {
      createFeed(() => [], frame);
      dispose();
    });
    await flushMicrotasks();
    expect(subscribe).not.toHaveBeenCalled();
  });

  it("reports a subscription that could not be started", async () => {
    subscribe.mockRejectedValue("denied");
    await createRoot(async (dispose) => {
      const feed = createFeed(() => [{ kind: "allIndices" }], frame);
      await flushMicrotasks();
      expect(feed.fatal()?.message).toBe("denied");
      expect(feed.errorFor({ kind: "allIndices" })?.message).toBe("denied");
      dispose();
    });
  });
});
