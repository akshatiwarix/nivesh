import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { prov, quote } from "../../data/fixtures";
import { pinClock, unpinClock } from "../../data/testClock";
import type { HubUpdateDto, SourcedQuoteDto } from "../../ipc";
import { subscribeQuotes } from "../../ipc";
import type { View } from "../types";
import { QuoteView } from "./QuoteView";

vi.mock("../../ipc", () => ({
  inTauri: () => true,
  subscribeQuotes: vi.fn(),
  toIpcError: (e: unknown) => ({ code: "INTERNAL", message: String(e), retryable: false }),
}));

const subscribe = vi.mocked(subscribeQuotes);
const tick = () => new Promise<void>((r) => setTimeout(r, 40));

function sourced(over: Partial<SourcedQuoteDto["quote"]> = {}, provenance = prov()) {
  return {
    quote: { ...quote("RELIANCE", 1359.3, 1367), ...over },
    provenance,
  } satisfies SourcedQuoteDto;
}

describe("QuoteView", () => {
  let dispose: (() => void) | undefined;
  let root: HTMLElement;
  let push: (u: HubUpdateDto) => void;
  let stop: () => void;

  function mount(
    view: Extract<View, { kind: "quote" }> = { kind: "quote", exchange: "NSE", symbol: "RELIANCE" },
  ) {
    root = document.createElement("div");
    document.body.append(root);
    dispose = render(() => <QuoteView view={view} />, root);
  }

  beforeEach(() => {
    pinClock();
    stop = vi.fn();
    subscribe.mockReset();
    subscribe.mockImplementation(async (_t, onUpdate) => {
      push = onUpdate;
      return stop;
    });
  });

  afterEach(() => {
    unpinClock();
    dispose?.();
    document.body.innerHTML = "";
  });

  const text = () => root.textContent ?? "";

  it("subscribes to this instrument's quote topic and shows a loading state", async () => {
    mount();
    await tick();
    expect(subscribe).toHaveBeenCalledWith(
      [{ kind: "quote", exchange: "NSE", symbol: "RELIANCE" }],
      expect.any(Function),
    );
    expect(text()).toContain("Loading quote");
  });

  it("shows LTP, change, OHLC, previous close, volume and a DELAYED badge with source and time", async () => {
    mount();
    await tick();
    push({
      kind: "quotes",
      quotes: [
        sourced({
          open: 1370,
          high: 1373,
          low: 1354.5,
          volume: 5_909_374,
          week52High: 1611.8,
          week52Low: 1152.2,
        }),
      ],
    });
    await tick();
    expect(root.querySelector(".quote__ltp")?.textContent).toBe("1,359.30");
    expect(text()).toContain("▼ -7.70");
    expect(text()).toContain("-0.56%");
    expect(root.querySelector(".quote__price .down")).not.toBeNull();
    for (const want of ["1,370.00", "1,373.00", "1,354.50", "1,367.00", "59,09,374"]) {
      expect(text()).toContain(want);
    }
    expect(text()).toContain("1,152.20 – 1,611.80");
    expect(root.querySelector(".badge")?.textContent).toBe("DELAYED");
    expect(text()).toContain("NSE 11:14:00 IST");
    expect(text()).toContain("RELIANCE Limited"); // the company name from the quote
    expect(root.querySelector(".badge--live")).toBeNull();
  });

  it("places the 52-week marker proportionally", async () => {
    mount();
    await tick();
    push({
      kind: "quotes",
      quotes: [sourced({ ltp: 150, week52Low: 100, week52High: 200 })],
    });
    await tick();
    const mark = root.querySelector<HTMLElement>(".quote__mark");
    expect(mark?.style.left).toBe("50%");
  });

  it("shows price bands when the source has them, and dashes when it doesn't", async () => {
    mount();
    await tick();
    push({ kind: "quotes", quotes: [sourced({ lowerBand: 1230.3, upperBand: 1503.7 })] });
    await tick();
    expect(text()).toContain("1,230.30 – 1,503.70");
    push({ kind: "quotes", quotes: [sourced({ lowerBand: null, upperBand: null })] });
    await tick();
    expect(text()).not.toContain("1,230.30");
    expect(root.querySelector(".quote__grid")?.textContent).toContain("Price band—");
  });

  it("missing fields render as dashes, not NaN or blank", async () => {
    mount();
    await tick();
    push({
      kind: "quotes",
      quotes: [
        sourced({
          open: null,
          high: null,
          low: null,
          volume: null,
          week52High: null,
          week52Low: null,
        }),
      ],
    });
    await tick();
    expect(text()).not.toMatch(/NaN|undefined|null/);
    expect(root.querySelector(".quote__mark")).toBeNull();
    expect(root.querySelector(".quote__grid")?.textContent).toContain("Open—");
  });

  it("labels a fallback source and stale data honestly", async () => {
    mount();
    await tick();
    push({ kind: "quotes", quotes: [sourced({}, prov("yahoo"))] });
    await tick();
    expect(text()).toContain("YAHOO 11:14:00 IST");
    push({ kind: "quotes", quotes: [sourced({}, { ...prov("yahoo"), freshness: "STALE" })] });
    await tick();
    expect(root.querySelector(".badge")?.textContent).toBe("STALE");
    expect(root.querySelector(".badge--stale")).not.toBeNull();
  });

  it("updates in place", async () => {
    mount();
    await tick();
    push({ kind: "quotes", quotes: [sourced({ ltp: 100 })] });
    await tick();
    const ltp = root.querySelector(".quote__ltp");
    push({ kind: "quotes", quotes: [sourced({ ltp: 101 })] });
    await tick();
    expect(root.querySelector(".quote__ltp")).toBe(ltp);
    expect(ltp?.textContent).toBe("101.00");
  });

  it("says when the symbol isn't found, and shows other failures inline", async () => {
    mount({ kind: "quote", exchange: "NSE", symbol: "ZZZZ" });
    await tick();
    push({
      kind: "failed",
      topic: { kind: "quote", exchange: "NSE", symbol: "ZZZZ" },
      error: { code: "NOT_FOUND", message: "not found", retryable: false },
    });
    await tick();
    expect(text()).toContain("ZZZZ was not found on NSE");
  });

  it("keeps the last quote and says the refresh failed", async () => {
    mount();
    await tick();
    push({ kind: "quotes", quotes: [sourced()] });
    push({
      kind: "failed",
      topic: { kind: "quote", exchange: "NSE", symbol: "RELIANCE" },
      error: { code: "UNAVAILABLE", message: "blocked by the provider", retryable: true },
    });
    await tick();
    expect(root.querySelector(".quote__ltp")?.textContent).toBe("1,359.30");
    expect(root.querySelector("[role=alert]")?.textContent).toContain("blocked by the provider");
  });

  it("works for BSE symbols", async () => {
    mount({ kind: "quote", exchange: "BSE", symbol: "500325" });
    await tick();
    expect(subscribe).toHaveBeenCalledWith(
      [{ kind: "quote", exchange: "BSE", symbol: "500325" }],
      expect.any(Function),
    );
    push({
      kind: "quotes",
      quotes: [{ quote: { ...quote("500325", 1300), exchange: "BSE" }, provenance: prov("yahoo") }],
    });
    await tick();
    expect(root.querySelector(".view__dim")?.textContent).toBe("BSE");
  });

  it("unsubscribes when closed", async () => {
    mount();
    await tick();
    dispose?.();
    dispose = undefined;
    expect(stop).toHaveBeenCalledTimes(1);
  });
});
