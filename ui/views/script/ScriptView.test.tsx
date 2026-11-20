import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { scriptSave, scriptSource, scriptValidate } from "../../ipc";
import { PanelContext } from "../context";
import { insertAt, ScriptView, STARTER } from "./ScriptView";

vi.mock("../../ipc", () => ({
  inTauri: () => true,
  scriptSource: vi.fn(),
  scriptValidate: vi.fn(),
  scriptSave: vi.fn(),
  IpcFailure: class extends Error {},
}));

const tick = (ms = 20) => new Promise<void>((r) => setTimeout(r, ms));
const ok = { ok: true, line: null, message: null };

describe("ScriptView", () => {
  let dispose: (() => void) | undefined;
  let root: HTMLElement;
  const area = () => root.querySelector("textarea") as HTMLTextAreaElement;
  const key = (k: string, mods: KeyboardEventInit = {}) =>
    area().dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, ...mods }));

  beforeEach(() => {
    vi.mocked(scriptValidate).mockResolvedValue(ok);
    vi.mocked(scriptSave).mockResolvedValue(ok);
  });

  function mount(name = "TREND") {
    root = document.createElement("div");
    document.body.append(root);
    dispose = render(
      () => (
        <PanelContext.Provider
          value={{ panel: 0, open: () => {}, openBeside: () => {}, focused: () => true }}
        >
          <ScriptView view={{ kind: "script", name }} />
        </PanelContext.Provider>
      ),
      root,
    );
  }

  afterEach(() => {
    dispose?.();
    root.remove();
  });

  it("starts a new script from the starter, checked and focused", async () => {
    vi.mocked(scriptSource).mockResolvedValue(null);
    mount();
    await tick();
    expect(area().value).toBe(STARTER);
    expect(root.textContent).toContain("NEW");
    expect(root.textContent).toContain("COMPILES");
    expect(document.activeElement).toBe(area());
  });

  it("marks the error line in the gutter", async () => {
    vi.mocked(scriptSource).mockResolvedValue("fn on_bar(bar) {\n  let x = ;\n}");
    vi.mocked(scriptValidate).mockResolvedValue({
      ok: false,
      line: 2,
      message: "line 2: Expecting an expression",
    });
    mount();
    await tick();
    const marked = root.querySelectorAll(".script__line--error");
    expect(marked.length).toBe(1);
    expect(marked[0]?.textContent).toBe("2");
    expect(root.textContent).toContain("Expecting an expression");
  });

  it("Tab indents, Ctrl+S saves, Esc leaves the editor", async () => {
    vi.mocked(scriptSource).mockResolvedValue("fn on_bar(bar) { }");
    mount();
    await tick();
    area().setSelectionRange(0, 0);
    key("Tab");
    expect(area().value.startsWith("    fn on_bar")).toBe(true);
    expect(root.textContent).toContain("UNSAVED");
    key("s", { ctrlKey: true });
    await tick();
    expect(scriptSave).toHaveBeenCalledWith("TREND", "    fn on_bar(bar) { }");
    expect(root.textContent).toContain("Saved TREND");
    expect(root.textContent).not.toContain("UNSAVED");
    key("Escape");
    expect(document.activeElement).not.toBe(area());
  });

  it("insertAt replaces the selection", () => {
    expect(insertAt("abcd", 1, 3, "XY")).toEqual({ value: "aXYd", caret: 3 });
  });
});
