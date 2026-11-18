import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";
import { completionWords } from "../commands";
import { createHistory } from "../commands/history";
import { CommandBar } from "./CommandBar";

describe("CommandBar", () => {
  let dispose: (() => void) | undefined;
  afterEach(() => {
    dispose?.();
    document.body.innerHTML = "";
  });

  function setup(onSubmit: (l: string) => string | undefined = vi.fn()) {
    const onBack = vi.fn();
    const history = createHistory();
    const root = document.createElement("div");
    document.body.append(root);
    dispose = render(
      () => (
        <CommandBar
          history={history}
          words={completionWords()}
          onSubmit={onSubmit}
          onBack={onBack}
        />
      ),
      root,
    );
    const input = root.querySelector("input") as HTMLInputElement;
    const type = (value: string) => {
      input.value = value;
      input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    };
    const press = (key: string, init: KeyboardEventInit = {}) => {
      const e = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init });
      input.dispatchEvent(e);
      return e;
    };
    return { root, input, type, press, onBack, history };
  }

  it("focuses itself on mount", () => {
    const { input } = setup();
    expect(document.activeElement).toBe(input);
  });

  it("Enter submits and clears on success", () => {
    const submit = vi.fn<(l: string) => string | undefined>();
    const { input, type, press, history } = setup(submit);
    type("nifty");
    press("Enter");
    expect(submit).toHaveBeenCalledWith("nifty");
    expect(input.value).toBe("");
    expect(history.entries()).toEqual(["nifty"]);
  });

  it("shows an inline error and keeps the text on failure", () => {
    const { root, input, type, press } = setup(() => "Unknown function");
    type("x zz");
    press("Enter");
    expect(root.querySelector("[role=alert]")?.textContent).toBe("Unknown function");
    expect(input.value).toBe("x zz");
    type("x z");
    expect(root.querySelector("[role=alert]")).toBeNull();
  });

  it("Esc clears first, then goes back", () => {
    const { input, type, press, onBack } = setup();
    type("abc");
    press("Escape");
    expect(input.value).toBe("");
    expect(onBack).not.toHaveBeenCalled();
    press("Escape");
    expect(onBack).toHaveBeenCalledTimes(1);
  });

  it("arrow keys walk history", () => {
    const { input, type, press } = setup();
    type("A1");
    press("Enter");
    type("B2");
    press("Enter");
    press("ArrowUp");
    expect(input.value).toBe("B2");
    press("ArrowUp");
    expect(input.value).toBe("A1");
    press("ArrowDown");
    press("ArrowDown");
    expect(input.value).toBe("");
  });

  it("Tab completes, but only claims the key when text is typed", () => {
    const { input, type, press } = setup();
    expect(press("Tab").defaultPrevented).toBe(false);
    type("he");
    expect(press("Tab").defaultPrevented).toBe(true);
    expect(input.value).toBe("HELP ");
  });

  it("printable keys elsewhere focus the bar", () => {
    const { input } = setup();
    const other = document.createElement("button");
    document.body.append(other);
    other.focus();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "r" }));
    expect(document.activeElement).toBe(input);
  });
});
