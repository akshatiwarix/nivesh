import { type Component, For } from "solid-js";
import "./KeyHint.css";

export interface KeyHintProps {
  /** One or more equivalent keys, e.g. ["F1", "Ctrl+1"]. */
  keys: readonly string[];
  /** What the key does, e.g. "HELP". */
  label?: string;
}

/** Renders key caps and an optional label: `[F1] HELP`. */
export const KeyHint: Component<KeyHintProps> = (props) => {
  return (
    <span class="keyhint">
      <For each={props.keys}>
        {(key, i) => (
          <>
            {i() > 0 && <span class="keyhint__or">/</span>}
            <kbd class="keyhint__key">{key}</kbd>
          </>
        )}
      </For>
      {props.label && <span class="keyhint__label">{props.label}</span>}
    </span>
  );
};
