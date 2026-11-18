import { render } from "solid-js/web";
import { afterEach, describe, expect, it } from "vitest";
import { App } from "./App";

describe("App shell", () => {
  let dispose: (() => void) | undefined;
  let root: HTMLElement;
  const mount = () => {
    root = document.createElement("div");
    document.body.append(root);
    dispose = render(() => <App />, root);
  };
  const key = (init: KeyboardEventInit) =>
    window.dispatchEvent(new KeyboardEvent("keydown", { cancelable: true, ...init }));
  const panels = () => root.querySelectorAll(".panel").length;

  afterEach(() => {
    dispose?.();
    document.body.innerHTML = "";
  });

  it("renders header, one panel and status bar", () => {
    mount();
    expect(root.textContent).toContain("NIVESH");
    expect(root.textContent).toContain("IST");
    expect(panels()).toBe(1);
  });

  it("switches layout and moves the focus highlight with Tab", () => {
    mount();
    key({ key: "4", code: "Digit4", altKey: true });
    expect(panels()).toBe(4);
    key({ key: "Tab" });
    const focused = root.querySelectorAll(".panel--focused");
    expect(focused.length).toBe(1);
    expect(root.querySelectorAll(".panel")[1]).toBe(focused[0]);
    key({ key: "2", code: "Digit2", altKey: true });
    expect(panels()).toBe(2);
  });

  it("typing RELIANCE + Enter opens the quote view, Esc goes back", () => {
    mount();
    const input = root.querySelector("input") as HTMLInputElement;
    const press = (k: string) =>
      input.dispatchEvent(
        new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true }),
      );
    input.value = "reliance";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    press("Enter");
    expect(root.querySelector(".panel__name")?.textContent).toBe("RELIANCE · NSE");
    expect(root.querySelector("[data-view=quote]")).not.toBeNull();
    press("Escape");
    expect(root.querySelector("[data-view=empty]")).not.toBeNull();
  });

  it("F1 and HELP open the help view listing commands and keys", () => {
    mount();
    key({ key: "F1", code: "F1" });
    const help = root.querySelector("[data-view=help]");
    expect(help).not.toBeNull();
    expect(help?.textContent).toContain("Commands and keys");
    expect(help?.textContent).toContain("Focus next panel");
    // Esc is handled by the focused command bar.
    document.activeElement?.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }),
    );
    expect(root.querySelector("[data-view=empty]")).not.toBeNull();
  });

  it("? opens help only when no input has focus", () => {
    mount();
    // The command bar has focus on mount: `?` is text there.
    key({ key: "?", code: "Slash", shiftKey: true });
    expect(root.querySelector("[data-view=help]")).toBeNull();
    (document.activeElement as HTMLElement).blur();
    key({ key: "?", code: "Slash", shiftKey: true });
    expect(root.querySelector("[data-view=help]")).not.toBeNull();
  });
});
