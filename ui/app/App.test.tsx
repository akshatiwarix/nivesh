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
  const key = (init: KeyboardEventInit) => window.dispatchEvent(new KeyboardEvent("keydown", init));
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
    key({ key: "4", altKey: true });
    expect(panels()).toBe(4);
    key({ key: "Tab" });
    const focused = root.querySelectorAll(".panel--focused");
    expect(focused.length).toBe(1);
    expect(root.querySelectorAll(".panel")[1]).toBe(focused[0]);
    key({ key: "2", altKey: true });
    expect(panels()).toBe(2);
  });
});
