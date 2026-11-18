import { type Component, createResource, Show } from "solid-js";
import { Badge } from "../components/Badge";
import { KeyHint } from "../components/KeyHint";
import { Panel } from "../components/Panel";
import { appInfo, inTauri } from "../ipc";
import "./App.css";

export const App: Component = () => {
  // Outside the app (browser, tests) there is no backend to ask.
  const [info] = createResource(inTauri, () => appInfo());
  return (
    <main class="app">
      <Panel
        title="NIVESH"
        focused
        extra={
          <Show when={info.state === "ready" && info()}>
            {(i) => <Badge tone="neutral">{`v${i().version} ${i().build}`}</Badge>}
          </Show>
        }
      >
        <p class="app__welcome">
          Keyboard-driven terminal for Indian markets. <KeyHint keys={["F1"]} label="Help" />
        </p>
      </Panel>
    </main>
  );
};
