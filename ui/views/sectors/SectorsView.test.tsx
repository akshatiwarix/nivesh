import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { indexUpdate, prov } from "../../data/fixtures";
import { pinClock, unpinClock } from "../../data/testClock";
import type { IndexSnapshotDto, SectorsDto } from "../../ipc";
import { getSectors, indexSnapshot } from "../../ipc";
import { PanelContext, sendViewOp } from "../context";
import type { View } from "../types";
import { SectorsView, tilesFor } from "./SectorsView";

vi.mock("../../ipc", () => ({
  inTauri: () => true,
  getSectors: vi.fn(),
  indexSnapshot: vi.fn(),
  IpcFailure: class extends Error {},
}));

const tick = () => new Promise<void>((r) => setTimeout(r, 40));

const row = (index: string, title: string, d1: number | null) => ({
  index,
  title,
  last: 25_000,
  d1,
  w1: 1.2,
  m1: null,
  ytd: -3.4,
});

function sectors(provider = "yahoo", freshness: "DELAYED" | "STALE" = "DELAYED"): SectorsDto {
  return {
    rows: [row("NIFTY50", "NIFTY 50", 0.5), row("NIFTYIT", "NIFTY IT", -1.25)],
    provenance: prov(provider, freshness),
  };
}

function snapshot(code: string, symbols: string[]): IndexSnapshotDto {
  const u = indexUpdate(code, code, 100, symbols, "yahoo");
  if (u.kind !== "index") throw new Error("fixture");
  return u.snapshot;
}

describe("SectorsView", () => {
  let dispose: (() => void) | undefined;
  let root: HTMLElement;
  const opened: View[] = [];
  const beside: View[] = [];

  function mount() {
    root = document.createElement("div");
    document.body.append(root);
    dispose = render(
      () => (
        <PanelContext.Provider
          value={{
            panel: 0,
            open: (v) => opened.push(v),
            openBeside: (v) => beside.push(v),
            focused: () => true,
          }}
        >
          <SectorsView />
        </PanelContext.Provider>
      ),
      root,
    );
  }
  const text = () => root.textContent ?? "";

  beforeEach(() => {
    pinClock();
    vi.mocked(getSectors).mockReset().mockResolvedValue(sectors());
    vi.mocked(indexSnapshot)
      .mockReset()
      .mockImplementation(async (code) =>
        snapshot(code, code === "NIFTYIT" ? ["TCS", "INFY"] : ["RELIANCE", "HDFCBANK", "SBIN"]),
      );
    opened.length = 0;
    beside.length = 0;
  });
  afterEach(() => {
    unpinClock();
    dispose?.();
    document.body.innerHTML = "";
  });

  it("shows returns with arrows, dashes for missing periods, and a DELAYED label with source", async () => {
    mount();
    await tick();
    expect(text()).toContain("NIFTY IT");
    expect(text()).toContain("▼ -1.25%");
    expect(text()).toContain("▲ +0.50%");
    expect(text()).toContain("—"); // 1M is unknown
    expect(text()).toContain("DELAYED");
    expect(text()).toContain("YAHOO");
  });

  it("draws the heatmap of the first index and follows the selected row", async () => {
    mount();
    await tick();
    expect(indexSnapshot).toHaveBeenCalledWith("NIFTY50");
    expect(text()).toContain("NIFTY 50 CONSTITUENTS BY % CHANGE");
    expect(root.querySelector("canvas")?.getAttribute("aria-label")).toContain("NIFTY 50");
    sendViewOp(0, "down");
    await tick();
    expect(indexSnapshot).toHaveBeenLastCalledWith("NIFTYIT");
    expect(text()).toContain("NIFTY IT CONSTITUENTS BY % CHANGE");
  });

  it("Enter opens the index; after [ it opens the highlighted stock; Alt+Enter goes beside", async () => {
    mount();
    await tick();
    sendViewOp(0, "open");
    expect(opened.at(-1)).toEqual({ kind: "index", id: "NIFTY50", title: "NIFTY 50" });
    sendViewOp(0, "tabNext");
    sendViewOp(0, "tabNext");
    sendViewOp(0, "open");
    expect(opened.at(-1)).toEqual({ kind: "quote", exchange: "NSE", symbol: "HDFCBANK" });
    sendViewOp(0, "openBeside");
    expect(beside.at(-1)).toEqual({ kind: "quote", exchange: "NSE", symbol: "HDFCBANK" });
    // Moving to another index clears the tile cursor.
    sendViewOp(0, "down");
    sendViewOp(0, "open");
    expect(opened.at(-1)).toEqual({ kind: "index", id: "NIFTYIT", title: "NIFTY IT" });
  });

  it("labels STALE data as STALE and explains an index with no constituents", async () => {
    vi.mocked(getSectors).mockResolvedValue(sectors("yahoo", "STALE"));
    vi.mocked(indexSnapshot).mockResolvedValue({ ...snapshot("NIFTY50", []), constituents: [] });
    mount();
    await tick();
    expect(text()).toContain("STALE");
    expect(text()).toContain("No constituents are available for NIFTY 50");
  });

  it("shows an inline error when the sector table can't load", async () => {
    vi.mocked(getSectors).mockRejectedValue(new Error("Yahoo is rate limiting us"));
    mount();
    await tick();
    expect(text()).toContain("Yahoo is rate limiting us");
  });

  it("builds tiles from constituents in the index's order", () => {
    const tiles = tilesFor(snapshot("NIFTY50", ["A", "B"]));
    expect(tiles.map((t) => t.label)).toEqual(["A", "B"]);
    expect(tiles.map((t) => t.weight)).toEqual([null, null]);
    expect(tilesFor(undefined)).toEqual([]);
  });

  it("weights tiles by EOD turnover and says so", async () => {
    vi.mocked(indexSnapshot).mockImplementation(async (code) => ({
      ...snapshot(code, ["BIGCO", "SMALLCO"]),
      activity: {
        asOf: "2026-11-18",
        source: "NSE bhavcopy",
        turnover: { BIGCO: 600, SMALLCO: 300 },
      },
    }));
    mount();
    await tick();
    expect(text()).toContain("SIZE = TURNOVER (EOD 2026-11-18)");
    const tiles = tilesFor({
      ...snapshot("NIFTY50", ["BIGCO", "SMALLCO"]),
      activity: {
        asOf: "2026-11-18",
        source: "NSE bhavcopy",
        turnover: { BIGCO: 600, SMALLCO: 300 },
      },
    });
    expect(tiles.map((t) => t.weight)).toEqual([600, 300]);
  });
});
