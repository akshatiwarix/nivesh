import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { prov } from "../../data/fixtures";
import { pinClock, unpinClock } from "../../data/testClock";
import type { NewsDto } from "../../ipc";
import { getNews, openNewsLink } from "../../ipc";
import { PanelContext, sendViewOp } from "../context";
import type { View } from "../types";
import { NewsView, newsTime } from "./NewsView";

vi.mock("../../ipc", () => ({
  inTauri: () => true,
  getNews: vi.fn(),
  openNewsLink: vi.fn(),
  IpcFailure: class extends Error {},
}));

const tick = () => new Promise<void>((r) => setTimeout(r, 40));
// 2026-11-18 09:40 IST
const AT = Date.UTC(2026, 10, 18, 4, 10);

function news(over: Partial<NewsDto> = {}): NewsDto {
  return {
    query: null,
    items: [
      {
        title: "Reliance Industries posts record profit",
        source: "ET Markets",
        url: "https://example.com/a",
        publishedMs: AT,
        tags: ["RELIANCE"],
      },
      {
        title: "<b>Sensex</b> ends higher <script>alert(1)</script>",
        source: "Mint",
        url: "https://example.com/b",
        publishedMs: AT - 60_000,
        tags: [],
      },
    ],
    provenance: { ...prov("rss"), asOfMs: AT },
    ...over,
  };
}

describe("NewsView", () => {
  let dispose: (() => void) | undefined;
  let root: HTMLElement;

  function mount(view: Extract<View, { kind: "news" }> = { kind: "news" }) {
    root = document.createElement("div");
    document.body.append(root);
    dispose = render(
      () => (
        <PanelContext.Provider
          value={{ panel: 0, open: () => {}, openBeside: () => {}, focused: () => true }}
        >
          <NewsView view={view} />
        </PanelContext.Provider>
      ),
      root,
    );
  }
  const text = () => root.textContent ?? "";

  beforeEach(() => {
    pinClock();
    vi.mocked(getNews).mockReset().mockResolvedValue(news());
    vi.mocked(openNewsLink).mockReset().mockResolvedValue(undefined);
  });
  afterEach(() => {
    unpinClock();
    dispose?.();
    document.body.innerHTML = "";
  });

  it("lists headlines with source, IST time and tagged symbols, labelled DELAYED from RSS", async () => {
    mount();
    await tick();
    expect(getNews).toHaveBeenCalledWith(undefined, 100);
    expect(text()).toContain("Reliance Industries posts record profit");
    expect(text()).toContain("ET Markets");
    expect(text()).toContain("RELIANCE");
    expect(text()).toContain("DELAYED");
    expect(text()).toContain("RSS");
  });

  it("renders headlines as text only: markup in a title never becomes elements", async () => {
    mount();
    await tick();
    expect(root.querySelector("b")).toBeNull();
    expect(root.querySelector("script")).toBeNull();
    expect(root.querySelector("a")).toBeNull();
    expect(root.querySelector("img, iframe")).toBeNull();
    expect(text()).toContain("<b>Sensex</b> ends higher");
  });

  it("filters by symbol for SYMBOL NEWS", async () => {
    mount({ kind: "news", symbol: "RELIANCE" });
    await tick();
    expect(getNews).toHaveBeenCalledWith("RELIANCE", 100);
    expect(text()).toContain("NEWS · RELIANCE");
  });

  it("says so when nothing matches", async () => {
    vi.mocked(getNews).mockResolvedValue(news({ items: [] }));
    mount({ kind: "news", symbol: "TCS" });
    await tick();
    expect(text()).toContain("No recent headlines mention TCS");
  });

  it("Enter opens the selected link through the backend, and reports the outcome", async () => {
    mount();
    await tick();
    sendViewOp(0, "down");
    sendViewOp(0, "open");
    await tick();
    expect(openNewsLink).toHaveBeenCalledWith("https://example.com/b");
    expect(text()).toContain("Opened in your browser: Mint");

    vi.mocked(openNewsLink).mockRejectedValue(new Error("that link is not from the news feed"));
    sendViewOp(0, "up");
    sendViewOp(0, "open");
    await tick();
    expect(text()).toContain("Could not open the link: that link is not from the news feed");
  });

  it("shows STALE data as STALE, and errors inline", async () => {
    vi.mocked(getNews).mockResolvedValue(news({ provenance: prov("rss", "STALE") }));
    mount();
    await tick();
    expect(text()).toContain("STALE");
    dispose?.();
    document.body.innerHTML = "";
    vi.mocked(getNews).mockRejectedValue(new Error("all feeds failed"));
    mount();
    await tick();
    expect(text()).toContain("all feeds failed");
  });

  it("formats times in IST, with the date for older headlines", () => {
    expect(newsTime(AT, AT)).toBe("09:40");
    expect(newsTime(AT - 86_400_000, AT)).toBe("17 NOV 09:40");
  });
});
