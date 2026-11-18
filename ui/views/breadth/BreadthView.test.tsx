import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { prov } from "../../data/fixtures";
import { pinClock, unpinClock } from "../../data/testClock";
import type { BreadthDto, DmaBreadthDto, ProgressDto } from "../../ipc";
import { getBreadth, getDmaBreadth } from "../../ipc";
import { PanelContext, sendViewOp } from "../context";
import { BreadthView, pctText } from "./BreadthView";

vi.mock("../../ipc", () => ({
  inTauri: () => true,
  getBreadth: vi.fn(),
  getDmaBreadth: vi.fn(),
  IpcFailure: class extends Error {},
}));

const tick = () => new Promise<void>((r) => setTimeout(r, 40));

const breadth = (): BreadthDto => ({
  index: "NIFTY50",
  advances: 12,
  declines: 36,
  unchanged: 2,
  provenance: prov("nse"),
});

const dma = (universe = "NIFTY50"): DmaBreadthDto => ({
  universe,
  members: 50,
  periods: [
    { period: 20, measured: 50, above: 31, percent: 62 },
    { period: 50, measured: 50, above: 20, percent: 40 },
    { period: 200, measured: 0, above: 0, percent: null },
  ],
  membersAsOf: "2026-11-18",
  membersSource: "NSE Indices (niftyindices.com)",
  provenance: prov("yahoo"),
});

describe("BreadthView", () => {
  let dispose: (() => void) | undefined;
  let root: HTMLElement;

  function mount(index = "NIFTY50") {
    root = document.createElement("div");
    document.body.append(root);
    dispose = render(
      () => (
        <PanelContext.Provider
          value={{ panel: 0, open: () => {}, openBeside: () => {}, focused: () => true }}
        >
          <BreadthView view={{ kind: "brd", index }} />
        </PanelContext.Provider>
      ),
      root,
    );
  }
  const text = () => root.textContent ?? "";

  beforeEach(() => {
    pinClock();
    vi.mocked(getBreadth).mockReset().mockResolvedValue(breadth());
    vi.mocked(getDmaBreadth)
      .mockReset()
      .mockImplementation(async (u) => dma(u));
  });
  afterEach(() => {
    unpinClock();
    dispose?.();
    document.body.innerHTML = "";
  });

  it("shows advances, declines and unchanged with arrows, and the sources", async () => {
    mount();
    await tick();
    expect(getBreadth).toHaveBeenCalledWith("NIFTY50");
    expect(text()).toContain("▲ 12 advancing");
    expect(text()).toContain("▼ 36 declining");
    expect(text()).toContain("– 2 unchanged");
    expect(text()).toContain("DELAYED");
    expect(text()).toContain("NSE");
    expect(text()).toContain("YAHOO");
  });

  it("shows the share above each moving average, and says when one can't be measured", async () => {
    mount();
    await tick();
    expect(root.querySelector('[data-testid="dma-20"]')?.textContent).toContain("62.0%");
    expect(root.querySelector('[data-testid="dma-20"]')?.textContent).toContain("31 of 50");
    expect(root.querySelector('[data-testid="dma-50"]')?.textContent).toContain("40.0%");
    expect(root.querySelector('[data-testid="dma-200"]')?.textContent).toContain(
      "not enough history",
    );
    expect(root.querySelector('[data-testid="dma-note"]')?.textContent).toContain("2026-11-18");
  });

  it("reports progress while the sweep runs", async () => {
    let report: (p: ProgressDto) => void = () => {};
    let finish: (d: DmaBreadthDto) => void = () => {};
    vi.mocked(getDmaBreadth).mockImplementation((_u, onProgress) => {
      report = onProgress ?? (() => {});
      return new Promise((r) => {
        finish = r;
      });
    });
    mount();
    await tick();
    report({ done: 23, total: 50 });
    await tick();
    expect(text()).toContain("23 of 50 stocks");
    finish(dma());
    await tick();
    expect(text()).toContain("62.0%");
  });

  it("[ ] switches the sweep to NIFTY 100 and Alt+Right changes the breadth index", async () => {
    mount();
    await tick();
    sendViewOp(0, "tabNext");
    await tick();
    expect(getDmaBreadth).toHaveBeenLastCalledWith("NIFTY100", expect.any(Function));
    expect(text()).toContain("NIFTY 100");
    sendViewOp(0, "nextList");
    await tick();
    expect(getBreadth).toHaveBeenLastCalledWith("BANKNIFTY");
  });

  it("shows errors inline", async () => {
    vi.mocked(getBreadth).mockRejectedValue(new Error("no constituents"));
    vi.mocked(getDmaBreadth).mockRejectedValue(new Error("only 5 of 50 stocks"));
    mount();
    await tick();
    expect(text()).toContain("no constituents");
    expect(text()).toContain("only 5 of 50 stocks");
  });

  it("formats percentages", () => {
    expect(pctText(62)).toBe("62.0%");
    expect(pctText(null)).toBe("—");
  });
});
