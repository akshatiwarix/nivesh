// SETTINGS: what the app is set to, in one place, and the command that
// changes each. Safety settings are changed only through their own
// commands (LIVE, RISK, KILL), so their typed confirmations always apply.

import { type Component, createResource, For, Show } from "solid-js";
import { startupMs } from "../../app/onboardingState";
import { Badge } from "../../components/Badge";
import { DataPanel } from "../../components/DataPanel";
import { kill } from "../../data/risk";
import { appInfo, appPaths, inTauri, riskStatus } from "../../ipc";
import type { View } from "../types";
import "./settings.css";

interface Row {
  setting: string;
  value: string;
  change: string;
}

export const SettingsView: Component<{ view: Extract<View, { kind: "settings" }> }> = () => {
  const [info] = createResource(inTauri, () => appInfo());
  const [paths] = createResource(inTauri, () => appPaths());
  const [risk] = createResource(inTauri, () => riskStatus());

  const rows = (): Row[] => {
    const r = risk();
    const k = kill() ?? r?.kill;
    return [
      {
        setting: "Live trading",
        value: r ? (r.liveEnabled ? "ON" : "OFF (paper only)") : "…",
        change: "LIVE ON / LIVE OFF",
      },
      {
        setting: "Kill switch",
        value: k ? (k.engaged ? "ENGAGED" : "ready") : "…",
        change: "KILL (Ctrl+Shift+K; Ctrl+Alt+Shift+K from any app) / KILL RESET",
      },
      { setting: "Risk limits", value: "see RISK", change: "RISK SET <FIELD> <N>" },
      { setting: "Layout", value: "1, 2 or 4 panels", change: "Alt+1 / Alt+2 / Alt+4" },
      { setting: "Broker", value: "see the status bar", change: "LOGIN / LOGOUT" },
      { setting: "Onboarding", value: "shown on first run", change: "WELCOME" },
    ];
  };

  return (
    <div class="view settings" data-view="settings">
      <DataPanel provenance={null} title="SETTINGS">
        <table class="settings__table">
          <thead>
            <tr>
              <th>Setting</th>
              <th>Now</th>
              <th>Change with</th>
            </tr>
          </thead>
          <tbody>
            <For each={rows()}>
              {(row) => (
                <tr>
                  <td>{row.setting}</td>
                  <td>{row.value}</td>
                  <td>
                    <code>{row.change}</code>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
        <dl class="settings__facts">
          <Show when={info()}>
            {(i) => (
              <>
                <dt>Version</dt>
                <dd>
                  {i().version} <Badge tone="neutral">{i().build}</Badge> {i().os}/{i().arch}
                </dd>
              </>
            )}
          </Show>
          <Show when={startupMs() !== undefined}>
            <dt>Startup</dt>
            <dd>{startupMs()} ms to interactive</dd>
          </Show>
          <Show when={paths()}>
            {(p) => (
              <>
                <dt>Data</dt>
                <dd>
                  <code>{p().dataDir}</code>
                </dd>
                <dt>Logs</dt>
                <dd>
                  <code>{p().logDir}</code>
                </dd>
              </>
            )}
          </Show>
        </dl>
      </DataPanel>
    </div>
  );
};
