import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, type Mock, vi } from "vitest";
import { allIndicesUpdate, indexRow } from "../../data/fixtures";
import type { HubUpdateDto } from "../../ipc";
import { subscribeQuotes } from "../../ipc";
import { PanelContext, sendViewOp } from "../context";
import type { View } from "../types";
import { MktView } from "./MktView";

vi.mock("../../ipc", () => ({
  inTauri: () => true,
  subscribeQuotes: vi.fn(),
  toIpcError: (e: unknown) => ({ code: "INTERNAL", message: String(e), retryable: false }),
}));

const subscribe = vi.mocked(subscribeQuotes);
const tick = () => new Promise<void>((r) => setTimeout(r, 40));

describe("MktView", () => {
  let dispose: (() => void) | undefined;
  let root: HTMLElement;
  let push: (u: HubUpdateDto) => void;
  let stop: Mock<() => void>;
  let opened: View[];

  beforeEach(() => {
    opened = [];
    stop = vi.fn();
    subscribe.mockReset();
    subscribe.mockImplementation(async (_topics, onUpdate) => {
      push = onUpdate;
      return stop;
    });
    root = document.createElement("div");
    document.body.append(root);
    dispose = render(
      () => (
        <PanelContext.Provider
          value={{ panel: 0, open: (v) => opened.push(v), focused: () => true }}
        >
          <MktView />
        </PanelContext.Provider>
      ),
      root,
    );
  });

  afterEach(() => {
    dispose?.();
    document.body.innerHTML = "";
  });

  const rows = () => [...root.querySelectorAll("tbody tr")];

  it("subscribes to the dashboard topic and shows a loading state first", async () => {
    await tick();
    expect(subscribe).toHaveBeenCalledWith([{ kind: "allIndices" }], expect.any(Function));
    expect(root.textContent).toContain("Loading indices");
  });

  it("renders every index with change arrows and a provenance badge per row", async () => {
    await tick();
    push(
      allIndicesUpdate(
        indexRow("NIFTY50", "NIFTY 50", 25243.4),
        indexRow("SENSEX", "SENSEX", 82094.07, "yahoo"),
      ),
    );
    await tick();
    expect(rows()).toHaveLength(2);
    const nifty = rows()[0] as HTMLElement;
    expect(nifty.textContent).toContain("NIFTY 50");
    expect(nifty.textContent).toContain("25,243.40");
    expect(nifty.textContent).toContain("▲ +10.00");
    expect(nifty.textContent).toContain("11/39");
    expect(nifty.querySelector(".badge")?.textContent).toBe("DELAYED");
    expect(nifty.textContent).toContain("NSE 11:14:00 IST");
    // The row from another provider says so: sources are never blended silently.
    expect((rows()[1] as HTMLElement).textContent).toContain("YAHOO 11:14:00 IST");
    expect(root.querySelector(".badge--live")).toBeNull();
  });

  it("updates a row in place: same element, new text", async () => {
    await tick();
    push(allIndicesUpdate(indexRow("NIFTY50", "NIFTY 50", 25000)));
    await tick();
    const row = rows()[0];
    push(allIndicesUpdate(indexRow("NIFTY50", "NIFTY 50", 25100)));
    await tick();
    expect(rows()[0]).toBe(row);
    expect(row?.textContent).toContain("25,100.00");
  });

  it("shows STALE data as STALE", async () => {
    await tick();
    const row = indexRow("NIFTY50", "NIFTY 50", 25000);
    push(allIndicesUpdate({ ...row, provenance: { ...row.provenance, freshness: "STALE" } }));
    await tick();
    expect(rows()[0]?.querySelector(".badge")?.textContent).toBe("STALE");
    expect(rows()[0]?.querySelector(".badge--stale")).not.toBeNull();
  });

  it("selects with Alt+Down and opens the index view with Enter", async () => {
    await tick();
    push(
      allIndicesUpdate(indexRow("NIFTY50", "NIFTY 50", 1), indexRow("BANKNIFTY", "NIFTY BANK", 2)),
    );
    await tick();
    expect(rows()[0]?.classList.contains("table__row--selected")).toBe(true);
    sendViewOp(0, "down");
    await tick();
    expect(rows()[1]?.classList.contains("table__row--selected")).toBe(true);
    sendViewOp(0, "up");
    sendViewOp(0, "up"); // clamps at the top
    sendViewOp(0, "down");
    sendViewOp(0, "open");
    expect(opened).toEqual([{ kind: "index", id: "BANKNIFTY", title: "NIFTY BANK" }]);
  });

  it("shows a failed refresh inline and keeps the last data", async () => {
    await tick();
    push(allIndicesUpdate(indexRow("NIFTY50", "NIFTY 50", 25000)));
    push({
      kind: "failed",
      topic: { kind: "allIndices" },
      error: { code: "UNAVAILABLE", message: "blocked by the provider", retryable: true },
    });
    await tick();
    expect(rows()).toHaveLength(1);
    expect(root.querySelector("[role=alert]")?.textContent).toContain("blocked by the provider");
  });

  it("shows the error when there is no data at all", async () => {
    await tick();
    push({
      kind: "failed",
      topic: { kind: "allIndices" },
      error: { code: "UNAVAILABLE", message: "timed out", retryable: true },
    });
    await tick();
    expect(root.textContent).toContain("Indices unavailable: timed out");
  });

  it("unsubscribes when the view is closed and stops taking keys", async () => {
    await tick();
    dispose?.();
    dispose = undefined;
    expect(stop).toHaveBeenCalledTimes(1);
    expect(sendViewOp(0, "down")).toBe(false);
  });
});
