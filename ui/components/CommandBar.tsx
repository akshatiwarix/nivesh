import { type Component, createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { applyTab, completions } from "../commands/complete";
import type { History } from "../commands/history";
import { searchTarget } from "../commands/suggest";
import { createSymbolSearch, type SearchFn } from "../data/search";
import type { InstrumentDto } from "../ipc";
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
  /** Instrument search for suggestions. Without it the bar has no suggestions. */
  search?: SearchFn;
}

/** Rows shown at most; keeps the list a fixed, small height. */
export const MAX_SUGGESTIONS = 8;

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
  const [selected, setSelected] = createSignal(-1);
  // Suggestions are for typing; while walking history they would be in the way.
  const [browsing, setBrowsing] = createSignal(false);

  const target = createMemo(() => (browsing() ? undefined : searchTarget(text())));
  const suggestions = createSymbolSearch(() => target()?.query, props.search);
  // Only show suggestions that answer the text as it is now.
  const shown = createMemo(() => (target() ? suggestions().slice(0, MAX_SUGGESTIONS) : []));

  const edit = (value: string, fromHistory = false) => {
    setText(value);
    setError(undefined);
    setBrowsing(fromHistory);
    setSelected(-1);
    cycle = undefined;
  };

  const accept = (i: InstrumentDto): string | undefined => {
    const line = target()?.line(i) ?? `${i.exchange}:${i.symbol}`;
    const err = props.onSubmit(line);
    props.history.push(line);
    if (err) setError(err);
    else edit("");
    return err;
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
    // Chords with Alt/Ctrl/Meta belong to the keymap (Alt+Up/Down move the
    // view's selection). Handling them here would swallow them.
    if (e.altKey || e.ctrlKey || e.metaKey) return;
    switch (e.key) {
      case "Enter": {
        e.preventDefault();
        const line = text().trim();
        if (line === "") {
          props.onEmptyEnter?.();
          return;
        }
        // A suggestion the user has moved onto beats the typed text.
        const pick = shown()[selected()];
        if (pick) {
          accept(pick);
          break;
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
        if (shown().length > 0) {
          setSelected((i) => (i <= 0 ? shown().length - 1 : i - 1));
          break;
        }
        const prev = props.history.prev(text());
        if (prev !== undefined) edit(prev, true);
        break;
      }
      case "ArrowDown": {
        e.preventDefault();
        if (shown().length > 0) {
          setSelected((i) => (i + 1) % shown().length);
          break;
        }
        const next = props.history.next();
        if (next !== undefined) edit(next, true);
        break;
      }
      case "Tab": {
        // With nothing typed, Tab keeps its panel-cycling meaning.
        if (text().trim() === "" || e.shiftKey) return;
        e.preventDefault();
        // Tab takes the highlighted (or best) suggestion when there is one.
        const pick = shown()[Math.max(selected(), 0)];
        if (pick && completions(text(), props.words).candidates.length === 0) {
          edit(`${pick.symbol} `);
          break;
        }
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
      <Show when={shown().length > 0}>
        <div
          class="suggest"
          id="cmdbar-suggestions"
          role="listbox"
          aria-label="Instrument suggestions"
        >
          <For each={shown()}>
            {(item, i) => (
              <div
                id={`cmdbar-suggestion-${i()}`}
                class="suggest__row"
                classList={{ "suggest__row--selected": i() === selected() }}
                role="option"
                tabIndex={-1}
                aria-selected={i() === selected()}
                // mousedown, not click: the input must keep focus.
                onMouseDown={(e) => {
                  e.preventDefault();
                  accept(item);
                }}
              >
                <span class="suggest__symbol">{item.symbol}</span>
                <span class="suggest__name">{item.name}</span>
                <span class="suggest__tag">{item.exchange}</span>
              </div>
            )}
          </For>
        </div>
      </Show>
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
        role="combobox"
        aria-expanded={shown().length > 0}
        aria-controls="cmdbar-suggestions"
        aria-activedescendant={selected() >= 0 ? `cmdbar-suggestion-${selected()}` : undefined}
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
