import { describe, expect, it } from "vitest";
import { parse } from "../commands";
import { createFakeWatchlistBackend } from "../data/fakeWatchlists";
import { createWatchlistStore } from "../data/watchlists";
import { dispatch, runCommand, WL_USAGE } from "./dispatch";
import { createWorkspace } from "./workspace";

describe("dispatch", () => {
  it("RELIANCE opens a quote view, NIFTY an index view, in the focused panel", () => {
    const ws = createWorkspace({ layout: 2, focused: 1 });
    expect(dispatch(parse("reliance"), ws)).toBeUndefined();
    expect(ws.currentView(1)).toEqual({ kind: "quote", exchange: "NSE", symbol: "RELIANCE" });
    expect(dispatch(parse("nifty"), ws)).toBeUndefined();
    expect(ws.currentView(1)).toMatchObject({ kind: "index", title: "NIFTY 50" });
    expect(ws.currentView(0).kind).toBe("empty");
  });

  it("passes parse errors through without opening anything", () => {
    const ws = createWorkspace();
    expect(dispatch(parse("XX:ABC"), ws)).toMatch(/Unknown exchange/);
    expect(ws.currentView(0).kind).toBe("empty");
  });

  it("MKT opens the dashboard once, F2 does the same, and Esc goes back", () => {
    const ws = createWorkspace();
    expect(dispatch(parse("mkt"), ws)).toBeUndefined();
    expect(ws.currentView(0).kind).toBe("mkt");
    // Re-running it doesn't stack a second dashboard.
    dispatch(parse("MKT"), ws);
    expect(ws.back()).toBe(true);
    expect(ws.currentView(0).kind).toBe("empty");
  });

  it("a view can open another view in its own panel", () => {
    const ws = createWorkspace({ layout: 2, focused: 0 });
    ws.openIn(1, { kind: "mkt" });
    expect(ws.currentView(1).kind).toBe("mkt");
    expect(ws.currentView(0).kind).toBe("empty");
    ws.openIn(9, { kind: "mkt" }); // out of range: ignored
  });

  describe("charts", () => {
    it("SYMBOL CH [INTERVAL] [RANGE] opens a chart in the focused panel", () => {
      const ws = createWorkspace();
      expect(dispatch(parse("reliance ch"), ws)).toBeUndefined();
      expect(ws.currentView(0)).toEqual({
        kind: "chart",
        exchange: "NSE",
        symbol: "RELIANCE",
        interval: "1d",
      });
      dispatch(parse("bse:tcs ch 5m 1mo"), ws);
      expect(ws.currentView(0)).toEqual({
        kind: "chart",
        exchange: "BSE",
        symbol: "TCS",
        interval: "5m",
        range: "1mo",
      });
    });

    it("rejects an unknown interval or range with a message", () => {
      const ws = createWorkspace();
      expect(dispatch(parse("reliance ch 7m"), ws)).toMatch(/Unknown interval/);
      expect(dispatch(parse("reliance ch 5m 2y"), ws)).toMatch(/Unknown range/);
      expect(ws.currentView(0).kind).toBe("empty");
    });

    it("CH and F4 chart the instrument the panel is showing", () => {
      const ws = createWorkspace();
      expect(runCommand("ch", ws)).toMatch(/Open an instrument first/);
      dispatch(parse("infy"), ws);
      expect(runCommand("ch", ws, ["15M"])).toBeUndefined();
      expect(ws.currentView(0)).toMatchObject({ kind: "chart", symbol: "INFY", interval: "15m" });
      // From a chart, CH changes its interval.
      runCommand("ch", ws, ["1H", "6MO"]);
      expect(ws.currentView(0)).toMatchObject({ interval: "1h", range: "6mo" });
    });
  });

  describe("movers", () => {
    it("MOV opens movers for NIFTY 50 by default, or the named index", () => {
      const ws = createWorkspace();
      expect(dispatch(parse("mov"), ws)).toBeUndefined();
      expect(ws.currentView(0)).toEqual({ kind: "mov", index: "NIFTY50" });
      dispatch(parse("mov banknifty"), ws);
      expect(ws.currentView(0)).toEqual({ kind: "mov", index: "BANKNIFTY" });
      expect(dispatch(parse("mov sensex"), ws)).toMatch(/not an index with constituents/);
      expect(dispatch(parse("mov foo"), ws)).toMatch(/not an index with constituents/);
    });
  });

  describe("breadth and sectors", () => {
    it("SEC opens sectors once and BRD opens breadth for NIFTY 50 or a named index", () => {
      const ws = createWorkspace();
      expect(dispatch(parse("sec"), ws)).toBeUndefined();
      expect(ws.currentView(0).kind).toBe("sec");
      dispatch(parse("sec"), ws);
      expect(ws.back()).toBe(true);
      expect(ws.currentView(0).kind).toBe("empty");

      expect(dispatch(parse("brd"), ws)).toBeUndefined();
      expect(ws.currentView(0)).toEqual({ kind: "brd", index: "NIFTY50" });
      dispatch(parse("brd niftyit"), ws);
      expect(ws.currentView(0)).toEqual({ kind: "brd", index: "NIFTYIT" });
      expect(dispatch(parse("brd nope"), ws)).toMatch(/not an index with constituents/);
    });
  });

  describe("indicators", () => {
    it("ADD and RM need a chart in the focused panel", () => {
      const ws = createWorkspace();
      expect(dispatch(parse("add sma 50"), ws)).toMatch(/works on a chart/);
      expect(dispatch(parse("rm rsi"), ws)).toMatch(/works on a chart/);
      dispatch(parse("reliance"), ws);
      expect(dispatch(parse("add rsi"), ws)).toMatch(/works on a chart/);
    });

    it("on a chart with no listener it asks to try again instead of failing silently", () => {
      const ws = createWorkspace();
      dispatch(parse("reliance ch"), ws);
      expect(dispatch(parse("add rsi"), ws)).toMatch(/still loading/);
    });
  });

  describe("WL", () => {
    const setup = () => {
      const ws = createWorkspace();
      const backend = createFakeWatchlistBackend();
      const store = createWatchlistStore(backend);
      const wl = (...args: string[]) => runCommand("wl", ws, args, store);
      return { ws, backend, store, wl };
    };
    const symbols = (store: ReturnType<typeof createWatchlistStore>) =>
      store.activeList()?.items.map((i) => i.symbol);

    it("WL opens the watchlist view once", () => {
      const { ws, wl } = setup();
      expect(wl()).toBeUndefined();
      expect(ws.currentView(0).kind).toBe("wl");
      wl();
      expect(ws.back()).toBe(true);
      expect(ws.currentView(0).kind).toBe("empty");
    });

    it("WL ADD adds every symbol (NSE by default) and shows the list", async () => {
      const { ws, store, wl } = setup();
      expect(await wl("ADD", "TCS", "BSE:500325", "INFY")).toBeUndefined();
      expect(symbols(store)).toEqual(["TCS", "500325", "INFY"]);
      expect(store.activeList()?.items[1]?.exchange).toBe("BSE");
      expect(ws.currentView(0).kind).toBe("wl");
    });

    it("WL ADD reports a bad symbol as the error text", async () => {
      const { ws, wl } = setup();
      expect(await wl("ADD", "BADSYM")).toBe("BADSYM is not a listed NSE symbol");
      expect(ws.currentView(0).kind).toBe("empty");
    });

    it("WL RM removes symbols", async () => {
      const { store, wl } = setup();
      await wl("ADD", "TCS", "INFY");
      expect(await wl("RM", "TCS")).toBeUndefined();
      expect(symbols(store)).toEqual(["INFY"]);
      expect(await wl("REMOVE", "INFY")).toBeUndefined();
      expect(symbols(store)).toEqual([]);
    });

    it("WL NEW creates and activates; WL USE switches by name or number", async () => {
      const { store, wl } = setup();
      expect(await wl("NEW", "tech", "stocks")).toBeUndefined();
      expect(store.activeList()?.name).toBe("TECH STOCKS");
      expect(await wl("USE", "1")).toBeUndefined();
      expect(store.activeList()?.name).toBe("WATCHLIST");
      expect(await wl("USE", "TECH", "STOCKS")).toBeUndefined();
      expect(store.activeList()?.name).toBe("TECH STOCKS");
      expect(await wl("USE", "NOPE")).toBe("No watchlist named NOPE");
    });

    it("WL DEL needs the exact name and refuses to delete the last list", async () => {
      const { store, wl } = setup();
      await wl("NEW", "TECH");
      expect(await wl("DEL", "TEC")).toBe("No watchlist named TEC");
      expect(store.state.lists).toHaveLength(2);
      expect(await wl("DEL", "tech")).toBeUndefined();
      expect(store.state.lists.map((l) => l.name)).toEqual(["WATCHLIST"]);
      expect(await wl("DEL", "WATCHLIST")).toBe("the last watchlist can't be deleted");
    });

    it("explains missing arguments and unknown subcommands", () => {
      const { wl } = setup();
      expect(wl("ADD")).toMatch(/needs a symbol/);
      expect(wl("RM")).toMatch(/needs a symbol/);
      expect(wl("NEW")).toMatch(/needs a name/);
      expect(wl("USE")).toMatch(/list name or number/);
      expect(wl("DEL")).toMatch(/exact|name/);
      expect(wl("FROB")).toBe(WL_USAGE);
    });

    it("parses end to end from the command bar grammar", async () => {
      const { ws } = setup();
      // Through the parser: WL is a registered command, so args reach the handler.
      const parsed = parse("wl add tcs");
      expect(parsed).toEqual({ kind: "command", id: "wl", args: ["ADD", "TCS"] });
      expect(ws.currentView(0).kind).toBe("empty");
    });
  });
});
