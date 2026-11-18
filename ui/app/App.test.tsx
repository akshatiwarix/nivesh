import { render } from "solid-js/web";
import { afterEach, describe, expect, it } from "vitest";
import { App } from "./App";

describe("App", () => {
  let dispose: (() => void) | undefined;
  afterEach(() => {
    dispose?.();
    document.body.innerHTML = "";
  });

  it("renders the shell into a container", () => {
    const root = document.createElement("div");
    document.body.append(root);
    dispose = render(() => <App />, root);
    expect(root.textContent).toContain("NIVESH");
  });
});
