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
