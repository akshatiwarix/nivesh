import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";
import { completionWords } from "../commands";
import { createHistory } from "../commands/history";
import type { SearchFn } from "../data/search";
import type { InstrumentDto } from "../ipc";
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

describe("CommandBar suggestions", () => {
  let dispose: (() => void) | undefined;
  afterEach(() => {
    dispose?.();
    document.body.innerHTML = "";
  });

  const hit = (symbol: string, name: string, exchange = "NSE") => ({
    exchange,
    symbol,
    name,
    series: "EQ",
  });
  const wait = (ms = 70) => new Promise<void>((r) => setTimeout(r, ms));

  function setup(search: SearchFn, onSubmit: (l: string) => string | undefined = vi.fn()) {
    const history = createHistory(["MKT", "HELP"]);
    const root = document.createElement("div");
    document.body.append(root);
    dispose = render(
      () => (
        <CommandBar
          history={history}
          words={completionWords()}
          onSubmit={onSubmit}
          onBack={vi.fn()}
          search={search}
        />
      ),
      root,
    );
    const input = root.querySelector("input") as HTMLInputElement;
    const type = (value: string) => {
      input.value = value;
      input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    };
    const press = (key: string) =>
      input.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
    const rows = () => [...root.querySelectorAll(".suggest__row")];
    return { root, input, type, press, rows, history };
  }

  it("suggests matches as you type, with exchange tags", async () => {
    const search = vi
      .fn<SearchFn>()
      .mockResolvedValue([
        hit("RELIANCE", "Reliance Industries Limited"),
        hit("RELINFRA", "Reliance Infrastructure Limited"),
      ]);
    const { type, rows } = setup(search);
    type("relia");
    expect(rows()).toHaveLength(0); // debounced
    await wait();
    expect(search).toHaveBeenCalledTimes(1);
    expect(search).toHaveBeenCalledWith("relia");
    expect(rows().map((r) => r.querySelector(".suggest__symbol")?.textContent)).toEqual([
      "RELIANCE",
      "RELINFRA",
    ]);
    expect(rows()[0]?.querySelector(".suggest__tag")?.textContent).toBe("NSE");
  });

  it("debounces a burst of keystrokes into one search", async () => {
    const search = vi.fn<SearchFn>().mockResolvedValue([]);
    const { type } = setup(search);
    for (const t of ["r", "re", "rel", "reli"]) type(t);
    await wait();
    expect(search).toHaveBeenCalledTimes(1);
    expect(search).toHaveBeenCalledWith("reli");
  });

  it("ignores a slow answer to an older query", async () => {
    let releaseSlow: (v: InstrumentDto[]) => void = () => {};
    const search = vi.fn<SearchFn>((q) =>
      q === "re"
        ? new Promise((r) => {
            releaseSlow = r;
          })
        : Promise.resolve([hit("RELIANCE", "Reliance")]),
    );
    const { type, rows } = setup(search);
    type("re");
    await wait();
    type("rel");
    await wait();
    releaseSlow([hit("WRONG", "Late answer")]);
    await wait(10);
    expect(rows().map((r) => r.querySelector(".suggest__symbol")?.textContent)).toEqual([
      "RELIANCE",
    ]);
  });

  it("Down/Up move the highlight and Enter opens the highlighted instrument", async () => {
    const submit = vi.fn<(l: string) => string | undefined>();
    const search = vi
      .fn<SearchFn>()
      .mockResolvedValue([hit("RELIANCE", "R"), hit("RELINFRA", "I")]);
    const { type, press, rows, input } = setup(search, submit);
    type("rel");
    await wait();
    press("ArrowDown");
    press("ArrowDown");
    expect(rows()[1]?.classList.contains("suggest__row--selected")).toBe(true);
    expect(input.getAttribute("aria-activedescendant")).toBe("cmdbar-suggestion-1");
    press("ArrowUp");
    press("Enter");
    // The explicit exchange form: RELIANCE could otherwise be read as an alias.
    expect(submit).toHaveBeenCalledWith("NSE:RELIANCE");
    expect(input.value).toBe("");
    await wait();
    expect(rows()).toHaveLength(0);
  });

  it("Enter without moving onto a suggestion runs what was typed", async () => {
    const submit = vi.fn<(l: string) => string | undefined>();
    const search = vi.fn<SearchFn>().mockResolvedValue([hit("NIFTYBEES", "Nifty BeES")]);
    const { type, press } = setup(search, submit);
    type("nifty");
    await wait();
    press("Enter");
    expect(submit).toHaveBeenCalledWith("nifty");
  });

  it("Tab fills the best suggestion's symbol", async () => {
    const search = vi.fn<SearchFn>().mockResolvedValue([hit("ZOMATO", "Zomato Limited")]);
    const { type, press, input } = setup(search);
    type("zomat");
    await wait();
    press("Tab");
    expect(input.value).toBe("ZOMATO ");
  });

  it("mouse selection opens the instrument without losing input focus", async () => {
    const submit = vi.fn<(l: string) => string | undefined>();
    const search = vi.fn<SearchFn>().mockResolvedValue([hit("TCS", "Tata Consultancy")]);
    const { type, rows, input } = setup(search, submit);
    input.focus();
    type("tcs");
    await wait();
    const down = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
    rows()[0]?.dispatchEvent(down);
    expect(down.defaultPrevented).toBe(true);
    expect(submit).toHaveBeenCalledWith("NSE:TCS");
    expect(document.activeElement).toBe(input);
  });

  it("doesn't search for commands or explicit exchanges", async () => {
    const search = vi.fn<SearchFn>().mockResolvedValue([]);
    const { type } = setup(search);
    for (const t of ["help", "mkt", "BSE:500", ""]) {
      type(t);
      await wait();
    }
    expect(search).not.toHaveBeenCalled();
  });

  it("history browsing keeps Up/Down and shows no suggestions", async () => {
    const search = vi.fn<SearchFn>().mockResolvedValue([hit("X", "X")]);
    const { press, input, rows } = setup(search);
    press("ArrowUp");
    expect(input.value).toBe("HELP");
    press("ArrowUp");
    expect(input.value).toBe("MKT");
    await wait();
    expect(rows()).toHaveLength(0);
  });

  it("clears the suggestions with Esc and survives a failing search", async () => {
    const search = vi.fn<SearchFn>().mockRejectedValue(new Error("boom"));
    const { type, press, rows, input } = setup(search);
    type("abc");
    await wait();
    expect(rows()).toHaveLength(0);
    search.mockResolvedValue([hit("ABB", "ABB India")]);
    type("ab");
    await wait();
    expect(rows()).toHaveLength(1);
    press("Escape");
    await wait(10);
    expect(input.value).toBe("");
    expect(rows()).toHaveLength(0);
  });

  it("works without a search function (browser, tests)", () => {
    const history = createHistory();
    const root = document.createElement("div");
    document.body.append(root);
    dispose = render(
      () => (
        <CommandBar
          history={history}
          words={completionWords()}
          onSubmit={vi.fn()}
          onBack={vi.fn()}
        />
      ),
      root,
    );
    const input = root.querySelector("input") as HTMLInputElement;
    input.value = "rel";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    expect(root.querySelector(".suggest")).toBeNull();
  });
});

describe("CommandBar async commands", () => {
  let dispose: (() => void) | undefined;
  afterEach(() => {
    dispose?.();
    document.body.innerHTML = "";
  });

  function mountBar(onSubmit: (l: string) => Promise<string | undefined>) {
    const root = document.createElement("div");
    document.body.append(root);
    dispose = render(
      () => (
        <CommandBar
          history={createHistory()}
          words={completionWords()}
          onSubmit={onSubmit}
          onBack={vi.fn()}
        />
      ),
      root,
    );
    const input = root.querySelector("input") as HTMLInputElement;
    input.value = "wl add tcs";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    input.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }),
    );
    return { root, input };
  }

  it("clears at once and shows an error that arrives later", async () => {
    const { root, input } = mountBar(async () => "TCSX is not a listed NSE symbol");
    expect(input.value).toBe("");
    await new Promise<void>((r) => setTimeout(r, 10));
    expect(root.querySelector("[role=alert]")?.textContent).toBe("TCSX is not a listed NSE symbol");
  });

  it("shows nothing when the async command succeeds", async () => {
    const { root } = mountBar(async () => undefined);
    await new Promise<void>((r) => setTimeout(r, 10));
    expect(root.querySelector("[role=alert]")).toBeNull();
  });

  it("shows a rejected promise instead of dropping it", async () => {
    const { root } = mountBar(() => Promise.reject(new Error("boom")));
    await new Promise<void>((r) => setTimeout(r, 10));
    expect(root.querySelector("[role=alert]")?.textContent).toContain("boom");
  });
});

describe("CommandBar and modifier chords", () => {
  it("leaves Alt+arrows alone so view navigation reaches the keymap", () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(
      () => (
        <CommandBar
          history={createHistory(["MKT"])}
          words={completionWords()}
          onSubmit={vi.fn()}
          onBack={vi.fn()}
        />
      ),
      root,
    );
    const input = root.querySelector("input") as HTMLInputElement;
    for (const key of ["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Delete"]) {
      const e = new KeyboardEvent("keydown", {
        key,
        altKey: true,
        bubbles: true,
        cancelable: true,
      });
      input.dispatchEvent(e);
      expect(e.defaultPrevented, key).toBe(false);
    }
    // History navigation is unaffected without the modifier.
    const plain = new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true, cancelable: true });
    input.dispatchEvent(plain);
    expect(plain.defaultPrevented).toBe(true);
    expect(input.value).toBe("MKT");
    dispose();
    document.body.innerHTML = "";
  });
});
