import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it } from "vitest";
import { AS_OF, prov } from "../data/fixtures";
import { DataPanel, type ProvenanceInput } from "./DataPanel";

describe("DataPanel", () => {
  let dispose: (() => void) | undefined;
  afterEach(() => {
    dispose?.();
    document.body.innerHTML = "";
  });

  const NOW = AS_OF + 90_000; // 90 s after the data

  function mount(provenance: () => ProvenanceInput) {
    const root = document.createElement("div");
    document.body.append(root);
    dispose = render(
      () => (
        <DataPanel provenance={provenance()} title="Quote" now={() => NOW}>
          <p class="body">content</p>
        </DataPanel>
      ),
      root,
    );
    return root;
  }
  const bar = (root: HTMLElement) => root.querySelector(".datapanel__bar")?.textContent ?? "";

  it("labels delayed public data with source, as-of time and age", () => {
    const root = mount(() => prov("nse"));
    expect(root.querySelector(".badge")?.textContent).toBe("DELAYED");
    expect(root.querySelector(".badge--delayed")).not.toBeNull();
    expect(bar(root)).toContain("NSE 11:14:00 IST");
    expect(bar(root)).toContain("1m ago");
    expect(root.querySelector(".body")?.textContent).toBe("content");
    expect(root.querySelector(".datapanel__note")).toBeNull();
  });

  it("says NO DATA, not a freshness label, before anything has arrived", () => {
    const root = mount(() => null);
    expect(bar(root)).toContain("NO DATA");
    expect(root.querySelector(".badge--delayed, .badge--live, .badge--stale")).toBeNull();
  });

  it("flags stale data with a banner and the STALE badge", () => {
    const root = mount(() => prov("nse", "STALE"));
    expect(root.querySelector(".badge")?.textContent).toBe("STALE");
    expect(root.querySelector(".badge--stale")).not.toBeNull();
    expect(root.querySelector(".datapanel__note--stale")?.textContent).toContain("last known data");
    expect(root.querySelector("[data-freshness=STALE]")).not.toBeNull();
  });

  it("labels a mixed view by its least fresh part and lists every source", () => {
    const root = mount(() => [prov("nse"), prov("yahoo", "STALE"), prov("nse")]);
    expect(root.querySelector(".badge")?.textContent).toBe("STALE");
    expect(bar(root)).toContain("NSE + YAHOO");
  });

  it("uses the oldest as-of time of a mixed view", () => {
    const older = { ...prov("yahoo"), asOfMs: AS_OF - 3600_000 };
    const root = mount(() => [prov("nse"), older]);
    expect(bar(root)).toContain("10:14:00 IST");
    expect(bar(root)).toContain("1h ago");
  });

  it("never shows LIVE for public sources", () => {
    const root = mount(() => [prov("nse"), prov("yahoo")]);
    expect(bar(root)).not.toContain("LIVE");
    expect(root.querySelector(".badge--live")).toBeNull();
  });

  it("notes a change of source for as long as the view stays open", () => {
    const [p, setP] = createSignal<ProvenanceInput>(prov("nse"));
    const root = mount(p);
    expect(root.querySelector(".datapanel__note")).toBeNull();
    setP(prov("yahoo"));
    expect(root.querySelector(".datapanel__note")?.textContent).toContain("NSE → YAHOO");
    expect(bar(root)).toContain("YAHOO");
    // A refresh from the same source doesn't clear or repeat the note.
    setP({ ...prov("yahoo"), fetchedAtMs: AS_OF + 5 });
    expect(root.querySelectorAll(".datapanel__note")).toHaveLength(1);
  });

  it("a freshness change from one source is not a source change", () => {
    const [p, setP] = createSignal<ProvenanceInput>(prov("nse"));
    const root = mount(p);
    setP(prov("nse", "STALE"));
    expect(root.textContent).not.toContain("Source changed");
  });
});
