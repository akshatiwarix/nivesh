import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AuditRowDto } from "../../ipc";
import { auditLog, auditVerify } from "../../ipc";
import { PanelContext, sendViewOp } from "../context";
import { AuditView, kindTone } from "./AuditView";

vi.mock("../../ipc", () => ({
  inTauri: () => true,
  auditLog: vi.fn(),
  auditVerify: vi.fn(),
  IpcFailure: class extends Error {},
}));

const tick = () => new Promise<void>((r) => setTimeout(r, 40));

const row = (seq: number, kind: string, payload = "{}"): AuditRowDto => ({
  seq,
  at: 1_795_150_800_000 + seq * 1000,
  kind,
  strategy: "TREND",
  strategyId: 1,
  payload,
  hash: "ab".repeat(32),
});

describe("AuditView", () => {
  let dispose: (() => void) | undefined;
  let root: HTMLElement;
  const text = () => root.textContent ?? "";

  function mount(view = { kind: "audit" as const }) {
    root = document.createElement("div");
    document.body.append(root);
    dispose = render(
      () => (
        <PanelContext.Provider
          value={{ panel: 3, open: () => {}, openBeside: () => {}, focused: () => true }}
        >
          <AuditView view={view} />
        </PanelContext.Provider>
      ),
      root,
    );
  }

  afterEach(() => {
    dispose?.();
    root.remove();
  });

  it("lists records with the chain status and the selected record's details", async () => {
    vi.mocked(auditLog).mockResolvedValue([
      row(3, "order.routed", '{"route":"paper"}'),
      row(2, "risk.rejected", '{"reason":"per-order cap"}'),
    ]);
    vi.mocked(auditVerify).mockResolvedValue({
      intact: true,
      rows: 3,
      brokenAt: null,
      reason: null,
    });
    mount();
    await tick();
    expect(text()).toContain("CHAIN INTACT · 3 RECORDS");
    expect(text()).toContain("risk.rejected");
    expect(root.querySelector(".audit__detail")?.textContent).toContain('"route":"paper"');
    sendViewOp(3, "down");
    await tick();
    expect(root.querySelector(".audit__detail")?.textContent).toContain("per-order cap");
  });

  it("passes the filter and shouts about a broken chain", async () => {
    vi.mocked(auditLog).mockResolvedValue([]);
    vi.mocked(auditVerify).mockResolvedValue({
      intact: false,
      rows: null,
      brokenAt: 7,
      reason: "hash mismatch",
    });
    mount({ kind: "audit", filter: "kill", strategy: "TREND" } as never);
    await tick();
    expect(auditLog).toHaveBeenLastCalledWith({ kind: "kill", strategy: "TREND" });
    expect(text()).toContain("CHAIN BROKEN AT #7");
    expect(text()).toContain("hash mismatch");
    expect(text()).toContain("No records match.");
  });

  it("colours failures and riskier changes", () => {
    expect(kindTone("kill.engaged")).toBe("down");
    expect(kindTone("order.ambiguous")).toBe("down");
    expect(kindTone("strategy.armed")).toBe("accent");
    expect(kindTone("fill")).toBeUndefined();
  });
});
