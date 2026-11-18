import type { JSX } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, describe, expect, it } from "vitest";
import { Badge } from "./Badge";
import { type Column, DataTable } from "./DataTable";
import { KeyHint } from "./KeyHint";
import { Panel } from "./Panel";

let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  document.body.innerHTML = "";
});

function mount(ui: () => JSX.Element): HTMLElement {
  const root = document.createElement("div");
  document.body.append(root);
  dispose = render(ui, root);
  return root;
}

interface Row {
  sym: string;
  chg: number;
}

const columns: Column<Row>[] = [
  { id: "sym", header: "Symbol", width: "12ch", value: (r) => r.sym },
  {
    id: "chg",
    header: "Chg %",
    align: "right",
    value: (r) => r.chg.toFixed(2),
    tone: (r) => (r.chg >= 0 ? "up" : "down"),
  },
];

describe("DataTable", () => {
  it("renders a header and one fixed-height row per item", () => {
    const rows = [
      { sym: "TCS", chg: 1.5 },
      { sym: "INFY", chg: -0.25 },
    ];
    const root = mount(() => (
      <DataTable label="t" columns={columns} rows={rows} selectedIndex={1} />
    ));
    const headers = [...root.querySelectorAll("th")].map((th) => th.textContent);
    expect(headers).toEqual(["Symbol", "Chg %"]);

    const bodyRows = root.querySelectorAll("tbody tr");
    expect(bodyRows).toHaveLength(2);
    expect(bodyRows[0]?.textContent).toBe("TCS1.50");
    expect(bodyRows[1]?.classList.contains("table__row--selected")).toBe(true);
    expect(bodyRows[0]?.querySelectorAll("td")[1]?.className).toContain("up");
    expect(bodyRows[1]?.querySelectorAll("td")[1]?.className).toContain("down");
    expect(root.querySelector("td")?.className).toContain("table__cell--left");
  });

  it("shows the empty text when there are no rows", () => {
    const root = mount(() => (
      <DataTable label="t" columns={columns} rows={[]} emptyText="NOTHING YET" />
    ));
    const empty = root.querySelector(".table__empty");
    expect(empty?.textContent).toBe("NOTHING YET");
    expect(empty?.getAttribute("colspan")).toBe("2");
  });
});

describe("Panel", () => {
  it("marks the focused panel", () => {
    const root = mount(() => (
      <Panel title="QUOTE" index={2} focused>
        body
      </Panel>
    ));
    const panel = root.querySelector("section");
    expect(panel?.classList.contains("panel--focused")).toBe(true);
    expect(panel?.getAttribute("aria-current")).toBe("true");
    expect(root.querySelector(".panel__index")?.textContent).toBe("2");
    expect(root.querySelector(".panel__body")?.textContent).toBe("body");
  });
});

describe("Badge and KeyHint", () => {
  it("renders a toned badge", () => {
    const root = mount(() => <Badge tone="delayed">Delayed</Badge>);
    expect(root.querySelector(".badge")?.className).toBe("badge badge--delayed");
  });

  it("renders equivalent keys and a label", () => {
    const root = mount(() => <KeyHint keys={["F1", "Ctrl+1"]} label="Help" />);
    expect([...root.querySelectorAll("kbd")].map((k) => k.textContent)).toEqual(["F1", "Ctrl+1"]);
    expect(root.querySelector(".keyhint__label")?.textContent).toBe("Help");
  });
});

describe("DataTable virtualisation", () => {
  const cols = [{ id: "n", header: "N", value: (r: { n: number }) => r.n }];
  const rowsOf = (n: number) => Array.from({ length: n }, (_, i) => ({ n: i }));

  function mountIn(rows: { n: number }[], selectedIndex?: number) {
    const host = document.createElement("div");
    host.className = "panel__body";
    document.body.append(host);
    const dispose = render(
      () => <DataTable label="t" columns={cols} rows={rows} selectedIndex={selectedIndex} />,
      host,
    );
    return { host, dispose };
  }
  const bodyRows = (host: HTMLElement) => host.querySelectorAll("tbody tr.table__row").length;

  it("renders every row of a short table", () => {
    const { host, dispose } = mountIn(rowsOf(60));
    expect(bodyRows(host)).toBe(60);
    expect(host.querySelector(".table__spacer")).toBeNull();
    dispose();
    host.remove();
  });

  it("renders only a window of a long table, with spacers standing in for the rest", () => {
    const { host, dispose } = mountIn(rowsOf(5_000));
    const drawn = bodyRows(host);
    expect(drawn).toBeGreaterThan(0);
    expect(drawn).toBeLessThan(100);
    const after = host.querySelector<HTMLElement>(".table__spacer");
    // Rows are 18 px (the default without layout): the spacer covers the rest.
    expect(Number.parseFloat(after?.style.height ?? "0")).toBeCloseTo((5_000 - drawn) * 18, 0);
    dispose();
    host.remove();
  });

  it("moves the window when the container scrolls", async () => {
    const { host, dispose } = mountIn(rowsOf(5_000));
    // The test DOM has no layout, so fake the scroll offset the browser would report.
    Object.defineProperty(host, "scrollTop", { value: 18 * 2_000, configurable: true });
    const table = host.querySelector("table");
    if (table) table.getBoundingClientRect = () => ({ top: -18 * 2_000, height: 0 }) as DOMRect;
    host.dispatchEvent(new Event("scroll"));
    await new Promise((r) => setTimeout(r, 0));
    const first = host.querySelector("tbody tr.table__row td")?.textContent;
    expect(Number(first)).toBeGreaterThan(1_900);
    expect(bodyRows(host)).toBeLessThan(100);
    dispose();
    host.remove();
  });
});
