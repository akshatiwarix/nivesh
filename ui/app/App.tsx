import { type Component, createResource, For, onCleanup, onMount, Show } from "solid-js";
import { completionWords, parse } from "../commands";
import { Badge } from "../components/Badge";
import { CommandBar } from "../components/CommandBar";
import { Panel } from "../components/Panel";
import { ProviderHealth } from "../components/ProviderHealth";
import { createProviderHealth } from "../data/health";
import { pushAlert, riskChanged, setKill } from "../data/risk";
import { createVisibilityReporter } from "../data/visibility";
import { appInfo, inTauri, onKillSwitchChanged, onRiskAlert, searchInstruments } from "../ipc";
import { resolveKey } from "../keys/keymap";
import { hasViewOps, sendViewOp } from "../views/context";
import { viewTitle } from "../views/types";
import { ViewHost } from "../views/ViewHost";
import "./App.css";
import { runAction } from "./actions";
import { createClock } from "./clock";
import { type CommandResult, dispatch } from "./dispatch";
import { Header } from "./Header";
import { MarketIndicator } from "./MarketIndicator";
import { createPersistence } from "./persistence";
import { createSafetyStatus, KillControl, SafetyBanner } from "./SafetyBar";
import { StatusBar } from "./StatusBar";
import { createWorkspace, PANEL_COUNT } from "./workspace";

const isTextInput = (el: Element | null) =>
  el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement;

const PANELS = Array.from({ length: PANEL_COUNT }, (_, i) => i);

export const App: Component = () => {
  // Outside the app (browser, tests) there is no backend to ask.
  const [info] = createResource(inTauri, () => appInfo());
  const now = createClock();
  const health = createProviderHealth();
  createVisibilityReporter();
  const ws = createWorkspace();
  const { history } = createPersistence(ws);
  const words = completionWords();

  const run = (line: string): CommandResult => dispatch(parse(line), ws);

  const onKey = (e: KeyboardEvent) => {
    // The command bar claims keys it handles itself (Tab with text, Esc, ...).
    if (e.defaultPrevented) return;
    const active = document.activeElement;
    const inInput = isTextInput(active);
    // A textarea (the script editor) counts as typing, never as an empty bar.
    const inputEmpty =
      !(active instanceof HTMLInputElement || active instanceof HTMLTextAreaElement) ||
      (active instanceof HTMLInputElement && active.value === "");
    const binding = resolveKey(e, inInput, undefined, inputEmpty);
    if (!binding) return;
    // A view key is only taken when the focused view listens for it, so it
    // never blocks typing or caret movement in the command bar for nothing.
    if (binding.action.type === "view" && !hasViewOps(ws.state.focused)) return;
    e.preventDefault();
    runAction(binding.action, ws);
  };
  onMount(() => window.addEventListener("keydown", onKey));
  onCleanup(() => window.removeEventListener("keydown", onKey));

  // Safety events: alerts are kept for the RISK view, and any kill-switch
  // change makes it refetch.
  if (inTauri()) {
    const stops = [
      onRiskAlert(pushAlert),
      onKillSwitchChanged((s) => {
        setKill(s);
        riskChanged();
      }),
    ];
    onCleanup(() => {
      for (const stop of stops) void stop.then((f) => f());
    });
  }

  const safety = createSafetyStatus();

  return (
    <div class="shell">
      <Header now={now()} status={<MarketIndicator />} />
      <SafetyBanner status={safety} />
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
                <ViewHost
                  view={ws.currentView(i)}
                  panel={i}
                  focused={ws.state.focused === i}
                  open={(view) => ws.openIn(i, view)}
                  openBeside={(view) => ws.openBeside(i, view)}
                />
              </Panel>
            </Show>
          )}
        </For>
      </main>
      <CommandBar
        history={history}
        search={inTauri() ? (q) => searchInstruments(q, 8) : undefined}
        words={words}
        onSubmit={run}
        onBack={() => ws.back()}
        onEmptyEnter={() => sendViewOp(ws.state.focused, "open")}
      />
      <StatusBar
        layout={ws.state.layout}
        focused={ws.state.focused}
        kill={<Show when={inTauri()}>{<KillControl />}</Show>}
        health={<ProviderHealth health={health} />}
        extra={
          <Show when={info.state === "ready" && info()}>
            {(i) => <Badge tone="neutral">{`v${i().version} ${i().build}`}</Badge>}
          </Show>
        }
      />
    </div>
  );
};
