import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { prov, quote } from "../../data/fixtures";
import { pinClock, unpinClock } from "../../data/testClock";
import type { HubUpdateDto } from "../../ipc";
import { PanelContext, sendViewOp } from "../context";
import type { View } from "../types";

const h = vi.hoisted(() => ({ subscribe: vi.fn() }));

vi.mock("../../ipc", async () => {
  const { createFakeWatchlistBackend } = await import("../../data/fakeWatchlists");
  return {
    ...createFakeWatchlistBackend(),
    inTauri: () => true,
    subscribeQuotes: h.subscribe,
    toIpcError: (e: unknown) => ({ code: "INTERNAL", message: String(e), retryable: false }),
  };
});

import { watchlistStore } from "../../data/watchlists";
import * as ipc from "../../ipc";
import { WatchlistView } from "./WatchlistView";

const tick = () => new Promise<void>((r) => setTimeout(r, 40));

describe("WatchlistView", () => {
  let dispose: (() => void) | undefined;
  let root: HTMLElement;
  let push: (u: HubUpdateDto) => void;
  let opened: View[];

  beforeEach(async () => {
    pinClock();
    opened = [];
    h.subscribe.mockReset();
    h.subscribe.mockImplementation(async (_t: unknown, onUpdate: (u: HubUpdateDto) => void) => {
      push = onUpdate;
      return () => {};
    });
    // Fresh backend, then reload the shared store from it.
    (ipc as unknown as { reset: () => void }).reset();
    await watchlistStore.load(true);
    await watchlistStore.add([
      { exchange: "NSE", symbol: "TCS" },
      { exchange: "NSE", symbol: "INFY" },
      { exchange: "BSE", symbol: "500325" },
    ]);
    h.subscribe.mockClear();

    root = document.createElement("div");
    document.body.append(root);
    dispose = render(
      () => (
        <PanelContext.Provider
          value={{
            panel: 0,
            open: (v) => opened.push(v),
            openBeside: () => {},
            focused: () => true,
          }}
        >
          <WatchlistView />
        </PanelContext.Provider>
      ),
      root,
    );
    await tick();
  });

  afterEach(() => {
    unpinClock();
    dispose?.();
    document.body.innerHTML = "";
  });

  const rows = () =>
    [...root.querySelectorAll("tbody tr")].filter((r) => !r.querySelector(".table__empty"));
  const symbols = () => rows().map((r) => r.querySelector("td")?.textContent);

  it("lists the active watchlist's symbols and subscribes to exactly those quotes", () => {
    expect(symbols()).toEqual(["TCS", "INFY", "BSE:500325"]);
    expect(h.subscribe).toHaveBeenCalledTimes(1);
    expect(h.subscribe.mock.calls[0]?.[0]).toEqual([
      { kind: "quote", exchange: "NSE", symbol: "TCS" },
      { kind: "quote", exchange: "NSE", symbol: "INFY" },
      { kind: "quote", exchange: "BSE", symbol: "500325" },
    ]);
    expect(root.querySelector(".wl__list--active")?.textContent).toContain("WATCHLIST");
  });

  it("shows live quotes with a provenance badge, and dashes until they arrive", async () => {
    expect(rows()[0]?.textContent).toContain("—");
    push({
      kind: "quotes",
      quotes: [{ quote: quote("TCS", 3095.5, 3110), provenance: prov("yahoo") }],
    });
    await tick();
    const tcs = rows()[0]?.textContent ?? "";
    expect(tcs).toContain("3,095.50");
    expect(tcs).toContain("▼ -14.50");
    expect(tcs).toContain("DELAYED");
    expect(tcs).toContain("YAHOO 11:14:00 IST");
    expect(rows()[1]?.textContent).toContain("—");
  });

  it("removes the selected row with the remove key", async () => {
    sendViewOp(0, "down");
    sendViewOp(0, "remove");
    await tick();
    expect(symbols()).toEqual(["TCS", "BSE:500325"]);
  });

  it("reorders the selected row and the selection follows it", async () => {
    sendViewOp(0, "down");
    sendViewOp(0, "down");
    sendViewOp(0, "moveUp");
    await tick();
    expect(symbols()).toEqual(["TCS", "BSE:500325", "INFY"]);
    expect(rows()[1]?.classList.contains("table__row--selected")).toBe(true);
    sendViewOp(0, "moveDown");
    await tick();
    expect(symbols()).toEqual(["TCS", "INFY", "BSE:500325"]);
    expect(rows()[2]?.classList.contains("table__row--selected")).toBe(true);
  });

  it("reordering doesn't resubscribe, but adding a symbol does", async () => {
    sendViewOp(0, "down");
    sendViewOp(0, "moveDown");
    await tick();
    expect(h.subscribe).toHaveBeenCalledTimes(1);
    await watchlistStore.add([{ exchange: "NSE", symbol: "WIPRO" }]);
    await tick();
    expect(h.subscribe).toHaveBeenCalledTimes(2);
    expect(h.subscribe.mock.calls[1]?.[0]).toHaveLength(4);
  });

  it("Enter opens the selected symbol's quote page", () => {
    sendViewOp(0, "down");
    sendViewOp(0, "open");
    expect(opened).toEqual([{ kind: "quote", exchange: "NSE", symbol: "INFY" }]);
    sendViewOp(0, "down");
    sendViewOp(0, "open");
    expect(opened[1]).toEqual({ kind: "quote", exchange: "BSE", symbol: "500325" });
  });

  it("switches between lists with the list keys and shows each list's symbols", async () => {
    await watchlistStore.create("TECH");
    await tick();
    expect(symbols()).toEqual([]);
    expect(root.textContent).toContain("Empty. Add a symbol with WL ADD");
    expect(root.querySelector(".wl__list--active")?.textContent).toContain("TECH");
    sendViewOp(0, "prevList");
    await tick();
    expect(symbols()).toEqual(["TCS", "INFY", "BSE:500325"]);
    sendViewOp(0, "nextList");
    await tick();
    expect(root.querySelector(".wl__list--active")?.textContent).toContain("TECH");
  });

  it("an empty list says how to add", async () => {
    await watchlistStore.remove(watchlistStore.activeList()?.items ?? []);
    await tick();
    expect(root.textContent).toContain("WL ADD <SYMBOL>");
  });

  it("shows a failed quote refresh inline", async () => {
    push({
      kind: "failed",
      topic: { kind: "quote", exchange: "NSE", symbol: "TCS" },
      error: { code: "UNAVAILABLE", message: "blocked by the provider", retryable: true },
    });
    await tick();
    expect(root.querySelector("[role=alert]")?.textContent).toContain("blocked by the provider");
  });

  it("every operation is reachable from the keyboard (listed in the hint)", () => {
    const hint = root.querySelector(".wl__hint")?.textContent ?? "";
    for (const key of ["select", "open", "reorder", "remove", "list"]) expect(hint).toContain(key);
    expect(ipc.inTauri()).toBe(true);
  });
});
