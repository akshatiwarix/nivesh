import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { indexUpdate } from "../../data/fixtures";
import type { HubUpdateDto } from "../../ipc";
import { subscribeQuotes } from "../../ipc";
import { PanelContext, sendViewOp } from "../context";
import type { View } from "../types";
import { IndexView } from "./IndexView";

vi.mock("../../ipc", () => ({
  inTauri: () => true,
  subscribeQuotes: vi.fn(),
  toIpcError: (e: unknown) => ({ code: "INTERNAL", message: String(e), retryable: false }),
}));

const subscribe = vi.mocked(subscribeQuotes);
const tick = () => new Promise<void>((r) => setTimeout(r, 40));

describe("IndexView", () => {
  let dispose: (() => void) | undefined;
  let root: HTMLElement;
  let push: (u: HubUpdateDto) => void;
  let opened: View[];

  beforeEach(() => {
    opened = [];
    subscribe.mockReset();
    subscribe.mockImplementation(async (_t, onUpdate) => {
      push = onUpdate;
      return () => {};
    });
    root = document.createElement("div");
    document.body.append(root);
    dispose = render(
      () => (
        <PanelContext.Provider
          value={{ panel: 0, open: (v) => opened.push(v), focused: () => true }}
        >
          <IndexView view={{ kind: "index", id: "NIFTY50", title: "NIFTY 50" }} />
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

  it("subscribes to the index topic", async () => {
    await tick();
    expect(subscribe).toHaveBeenCalledWith(
      [{ kind: "index", index: "NIFTY50" }],
      expect.any(Function),
    );
  });

  it("shows the index header, source badge and constituents", async () => {
    await tick();
    push(indexUpdate("NIFTY50", "NIFTY 50", 25243.4, ["RELIANCE", "TCS"]));
    await tick();
    const head = root.querySelector(".idx__head")?.textContent ?? "";
    expect(head).toContain("NIFTY 50");
    expect(head).toContain("25,243.40");
    expect(head).toContain("DELAYED");
    expect(head).toContain("NSE 11:14:00 IST");
    expect(rows()).toHaveLength(2);
    expect(rows()[0]?.textContent).toContain("RELIANCE");
    expect(rows()[0]?.textContent).toContain("1,000.00");
    expect(root.querySelector(".idx__stats")?.textContent).toContain("Adv/Dec");
  });

  it("updates constituents in place", async () => {
    await tick();
    push(indexUpdate("NIFTY50", "NIFTY 50", 25000, ["RELIANCE", "TCS"]));
    await tick();
    const reliance = rows()[0];
    push(indexUpdate("NIFTY50", "NIFTY 50", 25100, ["RELIANCE", "TCS"]));
    await tick();
    expect(rows()[0]).toBe(reliance);
    expect(root.querySelector(".idx__last")?.textContent).toBe("25,100.00");
  });

  it("explains when the source has no constituents (index level only)", async () => {
    await tick();
    push(indexUpdate("NIFTY50", "NIFTY 50", 25000, [], "yahoo"));
    await tick();
    expect(root.querySelector(".idx__head")?.textContent).toContain("YAHOO");
    expect(root.textContent).toContain("Constituents are not available from YAHOO");
  });

  it("opens the selected constituent's quote with Enter", async () => {
    await tick();
    push(indexUpdate("NIFTY50", "NIFTY 50", 25000, ["RELIANCE", "TCS", "INFY"]));
    await tick();
    sendViewOp(0, "down");
    sendViewOp(0, "down");
    sendViewOp(0, "open");
    expect(opened).toEqual([{ kind: "quote", exchange: "NSE", symbol: "INFY" }]);
  });

  it("shows a failure inline", async () => {
    await tick();
    push({
      kind: "failed",
      topic: { kind: "index", index: "NIFTY50" },
      error: { code: "UNAVAILABLE", message: "blocked by the provider", retryable: true },
    });
    await tick();
    expect(root.textContent).toContain("blocked by the provider");
  });
});
