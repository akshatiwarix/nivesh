import { type Component, createResource, For, onCleanup, onMount, Show } from "solid-js";
import { completionWords, parse } from "../commands";
import { createHistory } from "../commands/history";
import { Badge } from "../components/Badge";
import { CommandBar } from "../components/CommandBar";
import { Panel } from "../components/Panel";
import { appInfo, inTauri } from "../ipc";
import "./App.css";
import { createClock } from "./clock";
import { Header } from "./Header";
import { StatusBar } from "./StatusBar";
import { createWorkspace, PANEL_COUNT } from "./workspace";

const PANELS = Array.from({ length: PANEL_COUNT }, (_, i) => i);

export const App: Component = () => {
  // Outside the app (browser, tests) there is no backend to ask.
  const [info] = createResource(inTauri, () => appInfo());
  const now = createClock();
  const ws = createWorkspace();
  const history = createHistory();
  const words = completionWords();

  const run = (line: string): string | undefined => {
    const parsed = parse(line);
    return parsed.kind === "error" ? parsed.message : undefined;
  };

  // Interim layout/focus keys; replaced by the table-driven keymap later.
  const onKey = (e: KeyboardEvent) => {
    if (e.defaultPrevented) return;
    if (e.key === "Tab") {
      e.preventDefault();
      ws.cycleFocus(e.shiftKey ? -1 : 1);
    } else if (e.altKey && (e.key === "1" || e.key === "2" || e.key === "4")) {
      e.preventDefault();
      ws.setLayout(Number(e.key) as 1 | 2 | 4);
    }
  };
  onMount(() => window.addEventListener("keydown", onKey));
  onCleanup(() => window.removeEventListener("keydown", onKey));

  return (
    <div class="shell">
      <Header now={now()} />
      <main class="shell__grid" data-layout={ws.state.layout}>
        <For each={PANELS}>
          {(i) => (
            <Show when={i < ws.state.layout}>
              <Panel
                title="Empty"
                index={i + 1}
                focused={ws.state.focused === i}
                onMouseDown={() => ws.focus(i)}
              >
                <p class="shell__welcome">Type a command. F1 for help.</p>
              </Panel>
            </Show>
          )}
        </For>
      </main>
      <CommandBar history={history} words={words} onSubmit={run} onBack={() => {}} />
      <StatusBar
        layout={ws.state.layout}
        focused={ws.state.focused}
        extra={
          <Show when={info.state === "ready" && info()}>
            {(i) => <Badge tone="neutral">{`v${i().version} ${i().build}`}</Badge>}
          </Show>
        }
      />
    </div>
  );
};
