import { beforeEach, describe, expect, it } from "vitest";
import { createFakeWatchlistBackend, type FakeWatchlistBackend } from "./fakeWatchlists";
import { createWatchlistStore, parseItemRef } from "./watchlists";

describe("parseItemRef", () => {
  it.each([
    ["tcs", { exchange: "NSE", symbol: "TCS" }],
    ["BSE:500325", { exchange: "BSE", symbol: "500325" }],
    ["nse:m&m", { exchange: "NSE", symbol: "M&M" }],
    ["  infy ", { exchange: "NSE", symbol: "INFY" }],
  ])("%j", (text, want) => {
    expect(parseItemRef(text)).toEqual(want);
  });
});

describe("watchlist store", () => {
  let backend: FakeWatchlistBackend;
  let store: ReturnType<typeof createWatchlistStore>;

  beforeEach(() => {
    backend = createFakeWatchlistBackend();
    store = createWatchlistStore(backend);
  });

  const symbols = () => store.activeList()?.items.map((i) => i.symbol);

  it("loads once", async () => {
    expect(store.state.loaded).toBe(false);
    await store.load();
    await store.load();
    expect(store.state.loaded).toBe(true);
    expect(backend.calls.filter((c) => c === "watchlists")).toHaveLength(1);
    await store.load(true);
    expect(backend.calls.filter((c) => c === "watchlists")).toHaveLength(2);
  });

  it("adds, removes and reorders, replacing state with the backend's answer", async () => {
    await store.add([
      { exchange: "NSE", symbol: "TCS" },
      { exchange: "NSE", symbol: "INFY" },
      { exchange: "NSE", symbol: "WIPRO" },
    ]);
    expect(symbols()).toEqual(["TCS", "INFY", "WIPRO"]);
    await store.move({ exchange: "NSE", symbol: "WIPRO" }, -2);
    expect(symbols()).toEqual(["WIPRO", "TCS", "INFY"]);
    await store.remove([{ exchange: "NSE", symbol: "TCS" }]);
    expect(symbols()).toEqual(["WIPRO", "INFY"]);
  });

  it("keeps list identity across updates so the view doesn't rebuild", async () => {
    await store.load();
    const list = store.state.lists[0];
    await store.add([{ exchange: "NSE", symbol: "TCS" }]);
    expect(store.state.lists[0]).toBe(list);
  });

  it("creating a list activates it; lists can be found by name or number", async () => {
    await store.create("tech");
    expect(store.activeList()?.name).toBe("TECH");
    expect(store.findList("tech")?.name).toBe("TECH");
    expect(store.findList("1")?.name).toBe("WATCHLIST");
    expect(store.findList("2")?.name).toBe("TECH");
    expect(store.findList("9")).toBeUndefined();
    expect(store.findList("nope")).toBeUndefined();
  });

  it("cycles through lists and wraps", async () => {
    await store.create("A");
    await store.create("B");
    expect(store.activeList()?.name).toBe("B");
    await store.cycle(1);
    expect(store.activeList()?.name).toBe("WATCHLIST");
    await store.cycle(-1);
    expect(store.activeList()?.name).toBe("B");
  });

  it("cycling with a single list does nothing", async () => {
    await store.load();
    await store.cycle(1);
    expect(backend.calls.some((c) => c.startsWith("activate"))).toBe(false);
  });

  it("deleting the active list falls back to another", async () => {
    await store.create("TECH");
    const tech = store.activeList();
    if (tech) await store.delete(tech.id);
    expect(store.activeList()?.name).toBe("WATCHLIST");
  });

  it("backend refusals reject and leave the state alone", async () => {
    await store.add([{ exchange: "NSE", symbol: "TCS" }]);
    await expect(store.add([{ exchange: "NSE", symbol: "BADONE" }])).rejects.toThrow(
      "not a listed NSE symbol",
    );
    await expect(store.create("watchlist")).rejects.toThrow("already exists");
    expect(symbols()).toEqual(["TCS"]);
    await expect(store.delete(store.state.active)).rejects.toThrow("last watchlist");
  });

  it("a failed add stops the rest of a batch", async () => {
    await expect(
      store.add([
        { exchange: "NSE", symbol: "TCS" },
        { exchange: "NSE", symbol: "BADONE" },
        { exchange: "NSE", symbol: "INFY" },
      ]),
    ).rejects.toThrow();
    expect(symbols()).toEqual(["TCS"]);
  });
});
