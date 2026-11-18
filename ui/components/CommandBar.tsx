import { type Component, createSignal, onCleanup, onMount, Show } from "solid-js";
import { applyTab, completions } from "../commands/complete";
import type { History } from "../commands/history";
import "./CommandBar.css";

export interface CommandBarProps {
  history: History;
  /** Words offered by Tab completion. */
  words: readonly string[];
  /**
   * Runs a submitted line. Returns an error message to show inline, or
   * nothing on success (the bar then clears).
   */
  onSubmit: (line: string) => string | undefined;
  /** Esc pressed on an empty bar: go back. */
  onBack: () => void;
  /** Enter pressed on an empty bar: act on the focused view's selection. */
  onEmptyEnter?: () => void;
}

const isEditable = (el: Element | null) =>
  el instanceof HTMLInputElement ||
  el instanceof HTMLTextAreaElement ||
  el instanceof HTMLSelectElement
    ? true
    : el instanceof HTMLElement && el.isContentEditable;

/**
 * The command line. It captures typing from anywhere in the window: a
 * printable key focuses the bar and lands in it.
 */
export const CommandBar: Component<CommandBarProps> = (props) => {
  let input: HTMLInputElement | undefined;
  const [text, setText] = createSignal("");
  const [error, setError] = createSignal<string>();
  let cycle: { base: string; index: number } | undefined;

  const edit = (value: string) => {
    setText(value);
    setError(undefined);
    cycle = undefined;
  };

  const onWindowKey = (e: KeyboardEvent) => {
    if (e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey) return;
    if (e.key.length !== 1 || e.key === " ") return;
    if (isEditable(document.activeElement)) return;
    input?.focus();
  };
  onMount(() => {
    input?.focus();
    window.addEventListener("keydown", onWindowKey);
  });
  onCleanup(() => window.removeEventListener("keydown", onWindowKey));

  const onKeyDown = (e: KeyboardEvent) => {
    switch (e.key) {
      case "Enter": {
        e.preventDefault();
        const line = text().trim();
        if (line === "") {
          props.onEmptyEnter?.();
          return;
        }
        const err = props.onSubmit(line);
        props.history.push(line);
        if (err) {
          setError(err);
        } else {
          edit("");
        }
        break;
      }
      case "Escape":
        e.preventDefault();
        if (text() !== "" || error()) edit("");
        else props.onBack();
        break;
      case "ArrowUp": {
        e.preventDefault();
        const prev = props.history.prev(text());
        if (prev !== undefined) edit(prev);
        break;
      }
      case "ArrowDown": {
        e.preventDefault();
        const next = props.history.next();
        if (next !== undefined) edit(next);
        break;
      }
      case "Tab": {
        // With nothing typed, Tab keeps its panel-cycling meaning.
        if (text().trim() === "" || e.shiftKey) return;
        e.preventDefault();
        const r = applyTab(text(), props.words, cycle);
        setText(r.text);
        setError(undefined);
        cycle = r.cycle;
        break;
      }
    }
  };

  const hint = () => {
    const c = completions(text(), props.words).candidates;
    return c.length > 0 ? c.slice(0, 6).join("  ") : "";
  };

  return (
    <div class="cmdbar">
      <span class="cmdbar__prompt">&gt;</span>
      <input
        ref={input}
        class="cmdbar__input"
        type="text"
        value={text()}
        placeholder="Type a command (RELIANCE, NIFTY, HELP)"
        aria-label="Command bar"
        aria-invalid={error() ? "true" : undefined}
        spellcheck={false}
        autocomplete="off"
        autocapitalize="off"
        onInput={(e) => {
          props.history.reset();
          edit(e.currentTarget.value);
        }}
        onKeyDown={onKeyDown}
      />
      <Show when={error()} fallback={<span class="cmdbar__hint">{hint()}</span>}>
        <span class="cmdbar__error" role="alert">
          {error()}
        </span>
      </Show>
    </div>
  );
};
