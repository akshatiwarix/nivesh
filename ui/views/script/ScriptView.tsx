// SCRIPT <NAME>: a plain textarea editor for a sandboxed strategy script,
// with line numbers and an error gutter (no Monaco). It checks as you type
// (in the Rust sandbox), Ctrl+S saves, Ctrl+Enter checks now, Tab indents
// and Esc leaves the editor. BT <NAME> backtests it and PAPER START <NAME>
// paper-trades it, like any strategy.

import { type Component, createEffect, createSignal, For, on, onCleanup, Show } from "solid-js";
import { Badge } from "../../components/Badge";
import { DataPanel } from "../../components/DataPanel";
import { describeError } from "../../data/errors";
import { inTauri, type ScriptCheckDto, scriptSave, scriptSource, scriptValidate } from "../../ipc";
import { usePanel } from "../context";
import type { View } from "../types";
import "./script.css";

/** What a new script starts as. */
export const STARTER = `// Called on every closed bar. Return buy(qty), sell() or nothing.
// bar: t open high low close volume
// sma(n) ema(n) rsi(n) atr(n): () until warm
// highest(n) lowest(n): over the n bars before this one
// in_position() entry_price() bar_count()
fn on_bar(bar) {
    if !in_position() && bar.close > highest(20) {
        return buy(1);
    }
    if in_position() && bar.close < lowest(10) {
        return sell();
    }
}
`;

const CHECK_DELAY_MS = 400;
const INDENT = "    ";

/** Inserts `text` at the selection, returning the new value and caret. */
export function insertAt(value: string, start: number, end: number, text: string) {
  return { value: value.slice(0, start) + text + value.slice(end), caret: start + text.length };
}

export const ScriptView: Component<{ view: Extract<View, { kind: "script" }> }> = (props) => {
  const panel = usePanel();
  const [source, setSource] = createSignal<string>();
  const [check, setCheck] = createSignal<ScriptCheckDto>();
  const [status, setStatus] = createSignal<string>();
  const [dirty, setDirty] = createSignal(false);
  const [isNew, setIsNew] = createSignal(false);
  let area: HTMLTextAreaElement | undefined;
  let gutter: HTMLDivElement | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(timer));

  // Load the saved source (or the starter for a new name).
  createEffect(
    on(
      () => props.view.name,
      async (name) => {
        setSource(undefined);
        setCheck(undefined);
        setDirty(false);
        if (!inTauri()) {
          setSource(STARTER);
          return;
        }
        try {
          const saved = await scriptSource(name);
          setIsNew(saved === null);
          setSource(saved ?? STARTER);
          void runCheck();
        } catch (err) {
          setStatus(describeError(err));
        }
      },
    ),
  );

  // Take the focus when the panel is focused and the text is in.
  createEffect(() => {
    if (source() !== undefined && panel.focused()) queueMicrotask(() => area?.focus());
  });

  const runCheck = async () => {
    const text = source();
    if (text === undefined || !inTauri()) return;
    try {
      setCheck(await scriptValidate(text));
    } catch (err) {
      setStatus(describeError(err));
    }
  };

  const save = async () => {
    const text = source();
    if (text === undefined || !inTauri()) return;
    try {
      const result = await scriptSave(props.view.name, text);
      setCheck(result);
      if (result.ok) {
        setDirty(false);
        setIsNew(false);
        setStatus(`Saved ${props.view.name}. BT ${props.view.name} to backtest it.`);
      } else {
        setStatus("Not saved: fix the error first.");
      }
    } catch (err) {
      setStatus(describeError(err));
    }
  };

  const onInput = (value: string) => {
    setSource(value);
    setDirty(true);
    setStatus(undefined);
    clearTimeout(timer);
    timer = setTimeout(() => void runCheck(), CHECK_DELAY_MS);
  };

  const onKeyDown = (e: KeyboardEvent & { currentTarget: HTMLTextAreaElement }) => {
    const el = e.currentTarget;
    if (e.key === "Tab" && !e.ctrlKey && !e.altKey && !e.metaKey && !e.shiftKey) {
      e.preventDefault();
      const next = insertAt(el.value, el.selectionStart, el.selectionEnd, INDENT);
      el.value = next.value;
      el.setSelectionRange(next.caret, next.caret);
      onInput(next.value);
    } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
      e.preventDefault();
      void save();
    } else if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
      e.preventDefault();
      clearTimeout(timer);
      void runCheck();
    } else if (e.key === "Escape") {
      // Leave the editor; Tab and the panel keys work again.
      e.preventDefault();
      el.blur();
    }
  };

  const lines = () => (source() ?? "").split("\n").length;
  const errorLine = () => (check()?.ok === false ? (check()?.line ?? null) : null);

  return (
    <div class="view script" data-view="script">
      <DataPanel provenance={null} title={`SCRIPT · ${props.view.name}`}>
        <div class="script__head">
          <Show when={check()} fallback={<Badge tone="neutral">UNCHECKED</Badge>}>
            {(c) => (
              <Show when={c().ok} fallback={<Badge tone="down">ERROR</Badge>}>
                <Badge tone="up">COMPILES</Badge>
              </Show>
            )}
          </Show>
          <Show when={isNew()}>
            <Badge tone="neutral">NEW</Badge>
          </Show>
          <Show when={dirty()}>
            <Badge tone="accent">UNSAVED</Badge>
          </Show>
          <Badge tone="paper">PAPER UNTIL ARMED</Badge>
        </div>
        <Show when={source() !== undefined} fallback={<p class="strat__hint">Loading…</p>}>
          <div class="script__editor">
            <div class="script__gutter" ref={gutter} aria-hidden="true">
              <For each={Array.from({ length: lines() }, (_, i) => i + 1)}>
                {(n) => (
                  <div
                    class="script__line"
                    classList={{ "script__line--error": n === errorLine() }}
                  >
                    {n}
                  </div>
                )}
              </For>
            </div>
            <textarea
              ref={area}
              class="script__text"
              aria-label={`Script ${props.view.name}`}
              spellcheck={false}
              autocomplete="off"
              wrap="off"
              value={source() ?? ""}
              onInput={(e) => onInput(e.currentTarget.value)}
              onKeyDown={onKeyDown}
              onScroll={(e) => {
                if (gutter) gutter.scrollTop = e.currentTarget.scrollTop;
              }}
            />
          </div>
        </Show>
        <Show when={check()?.ok === false}>
          <p class="view__error" role="alert">
            {check()?.message}
          </p>
        </Show>
        <Show when={status()}>
          <p class="strat__hint" role="status">
            {status()}
          </p>
        </Show>
        <p class="strat__hint">
          Ctrl+S save · Ctrl+Enter check · Tab indent · Esc leave the editor · BT {props.view.name}{" "}
          backtests · PAPER START {props.view.name} paper-trades. Sandboxed: no files, network or
          clock; 200k operations and 50 ms per bar.
        </p>
      </DataPanel>
    </div>
  );
};
