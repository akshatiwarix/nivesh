import type { JSX, ParentComponent } from "solid-js";
import "./Panel.css";

export interface PanelProps {
  /** Short upper-case title, e.g. "RELIANCE · NSE". */
  title: string;
  /** 1-based panel number, shown in the title bar. */
  index?: number;
  focused?: boolean;
  /** Right-aligned title-bar content: badges, key hints. */
  extra?: JSX.Element;
  onMouseDown?: () => void;
}

/** A bordered, self-contained region with a title bar. */
export const Panel: ParentComponent<PanelProps> = (props) => {
  return (
    <section
      class="panel"
      classList={{ "panel--focused": props.focused === true }}
      aria-label={props.title}
      aria-current={props.focused ? "true" : undefined}
      onMouseDown={() => props.onMouseDown?.()}
    >
      <header class="panel__title">
        {props.index !== undefined && <span class="panel__index">{props.index}</span>}
        <span class="panel__name">{props.title}</span>
        <span class="panel__extra">{props.extra}</span>
      </header>
      <div class="panel__body">{props.children}</div>
    </section>
  );
};
