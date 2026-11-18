import type { Component, JSX } from "solid-js";
import { KeyHint } from "../components/KeyHint";

export interface StatusBarProps {
  layout: number;
  focused: number;
  /** Right-aligned extra content, e.g. app version. */
  extra?: JSX.Element;
}

/** Bottom bar: focus and layout state plus the most important key hints. */
export const StatusBar: Component<StatusBarProps> = (props) => (
  <footer class="shell__statusbar">
    <span>
      PANEL {props.focused + 1}/{props.layout}
    </span>
    <KeyHint keys={["F1"]} label="Help" />
    <KeyHint keys={["Tab"]} label="Next panel" />
    <KeyHint keys={["Alt+1", "Alt+2", "Alt+4"]} label="Layout" />
    <span class="shell__spacer">{props.extra}</span>
  </footer>
);
