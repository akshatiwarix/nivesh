import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { prov, quote } from "../../data/fixtures";
import { pinClock, unpinClock } from "../../data/testClock";
import type { MoversDto } from "../../ipc";
import { getMovers } from "../../ipc";
import { PanelContext, sendViewOp } from "../context";
import type { View } from "../types";
import { MoversView } from "./MoversView";

vi.mock("../../ipc", () => ({
  inTauri: () => true,
  getMovers: vi.fn(),
  IpcFailure: class extends Error {},
}));

const getMoversMock = vi.mocked(getMovers);
const tick = () => new Promise<void>((r) => setTimeout(r, 40));

function movers(over: Partial<MoversDto> = {}): MoversDto {
  return {
    index: "NIFTY50",
    gainers: [quote("TATASTEEL", 171.4, 163.2), quote("ADANIENT", 2345.6, 2260)],
    losers: [quote("BAJFINANCE", 712.3, 745)],
    mostActive: [quote("SBIN", 812, 790)],
    mostActiveProvenance: null,
    mostActiveRankedAsOf: null,
    provenance: prov("nse"),
    ...over,
  };
}

describe("MoversView", () => {
  let dispose: (() => void) | undefined;
  let root: HTMLElement;
  const opened: View[] = [];
  const beside: View[] = [];

  function mount(index = "NIFTY50") {
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
          <MoversView view={{ kind: "mov", index }} />
        </PanelContext.Provider>
      ),
      root,
    );
  }
  const text = () => root.textContent ?? "";
  const symbols = () =>
    [...root.querySelectorAll("tbody tr td:first-child")].map((td) => td.textContent);

  beforeEach(() => {
    pinClock();
    getMoversMock.mockReset();
    opened.length = 0;
    beside.length = 0;
  });
  afterEach(() => {
    unpinClock();
    dispose?.();
    document.body.innerHTML = "";
  });

  it("asks for the universe, shows gainers first, and labels the source DELAYED", async () => {
    getMoversMock.mockResolvedValue(movers());
    mount("BANKNIFTY");
    await tick();
    expect(getMoversMock).toHaveBeenCalledWith("BANKNIFTY", 15);
    expect(symbols()).toEqual(["TATASTEEL", "ADANIENT"]);
    expect(text()).toContain("DELAYED");
    expect(text()).toContain("NSE");
    expect(text()).toContain("NIFTY BANK");
  });

  it("[ and ] switch between gainers, losers and most active", async () => {
    getMoversMock.mockResolvedValue(movers());
    mount();
    await tick();
    sendViewOp(0, "tabNext");
    await tick();
    expect(symbols()).toEqual(["BAJFINANCE"]);
    sendViewOp(0, "tabNext");
    await tick();
    expect(symbols()).toEqual(["SBIN"]);
    sendViewOp(0, "tabNext");
    await tick();
    expect(symbols()).toEqual(["TATASTEEL", "ADANIENT"]);
    sendViewOp(0, "tabPrev");
    await tick();
    expect(symbols()).toEqual(["SBIN"]);
  });

  it("Enter opens the selected quote here and Alt+Enter in the next panel", async () => {
    getMoversMock.mockResolvedValue(movers());
    mount();
    await tick();
    sendViewOp(0, "down");
    sendViewOp(0, "open");
    expect(opened).toEqual([{ kind: "quote", exchange: "NSE", symbol: "ADANIENT" }]);
    sendViewOp(0, "up");
    sendViewOp(0, "openBeside");
    expect(beside).toEqual([{ kind: "quote", exchange: "NSE", symbol: "TATASTEEL" }]);
  });

  it("Alt+Left/Right change the index universe and refetch", async () => {
    getMoversMock.mockResolvedValue(movers());
    mount();
    await tick();
    sendViewOp(0, "nextList");
    await tick();
    expect(getMoversMock).toHaveBeenLastCalledWith("BANKNIFTY", 15);
  });

  it("explains an empty most-active list and shows STALE data as STALE", async () => {
    getMoversMock.mockResolvedValue(movers({ mostActive: [], provenance: prov("yahoo", "STALE") }));
    mount();
    await tick();
    sendViewOp(0, "tabPrev"); // wraps to MOST ACTIVE
    await tick();
    expect(text()).toContain("No volume data");
    expect(text()).toContain("STALE");
  });

  it("labels an EOD-ranked most-active list with its own provenance and date", async () => {
    getMoversMock.mockResolvedValue(
      movers({
        mostActive: [quote("SBIN", 812, 790)],
        mostActiveProvenance: prov("yahoo"),
        mostActiveRankedAsOf: "2026-11-18",
      }),
    );
    mount();
    await tick();
    sendViewOp(0, "tabPrev"); // MOST ACTIVE
    await tick();
    expect(text()).toContain("EOD 2026-11-18");
    // The badge switches to the rows' provider while the list is on.
    expect(text()).toContain("YAHOO");
    // And back to the panel's source on another tab.
    sendViewOp(0, "tabNext"); // GAINERS
    await tick();
    expect(text()).toContain("NSE");
  });

  it("shows an inline error, and keeps the last data when a refresh fails", async () => {
    getMoversMock.mockRejectedValue(new Error("NSE blocked and no fallback"));
    mount();
    await tick();
    expect(text()).toContain("NSE blocked and no fallback");
    expect(text()).toContain("NO DATA");
  });
});
