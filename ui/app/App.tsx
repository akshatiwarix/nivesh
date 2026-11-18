import { type Component, createResource, For, onCleanup, onMount, Show } from "solid-js";
import { completionWords, parse } from "../commands";
import { createHistory } from "../commands/history";
import { Badge } from "../components/Badge";
import { CommandBar } from "../components/CommandBar";
import { Panel } from "../components/Panel";
import { appInfo, inTauri } from "../ipc";
import { resolveKey } from "../keys/keymap";
import { viewTitle } from "../views/types";
import { ViewHost } from "../views/ViewHost";
import "./App.css";
import { runAction } from "./actions";
import { createClock } from "./clock";
import { dispatch } from "./dispatch";
import { Header } from "./Header";
import { StatusBar } from "./StatusBar";
import { createWorkspace, PANEL_COUNT } from "./workspace";

const isTextInput = (el: Element | null) =>
  el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement;

const PANELS = Array.from({ length: PANEL_COUNT }, (_, i) => i);

export const App: Component = () => {
  // Outside the app (browser, tests) there is no backend to ask.
  const [info] = createResource(inTauri, () => appInfo());
  const now = createClock();
  const ws = createWorkspace();
  const history = createHistory();
  const words = completionWords();

  const run = (line: string) => dispatch(parse(line), ws);

  const onKey = (e: KeyboardEvent) => {
    // The command bar claims keys it handles itself (Tab with text, Esc, ...).
    if (e.defaultPrevented) return;
    const binding = resolveKey(e, isTextInput(document.activeElement));
    if (!binding) return;
    e.preventDefault();
    runAction(binding.action, ws);
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
                title={viewTitle(ws.currentView(i))}
                index={i + 1}
                focused={ws.state.focused === i}
                onMouseDown={() => ws.focus(i)}
              >
                <ViewHost view={ws.currentView(i)} />
              </Panel>
            </Show>
          )}
        </For>
      </main>
      <CommandBar history={history} words={words} onSubmit={run} onBack={() => ws.back()} />
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
