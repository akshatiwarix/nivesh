import { type Component, createResource, Show } from "solid-js";
import { appInfo, inTauri } from "../ipc";

export const App: Component = () => {
  // Outside the app (browser, tests) there is no backend to ask.
  const [info] = createResource(inTauri, () => appInfo());
  return (
    <main class="app">
      <h1>NIVESH</h1>
      <Show when={info.state === "ready" && info()}>
        {(i) => (
          <p>
            v{i().version} ({i().build})
          </p>
        )}
      </Show>
    </main>
  );
};
