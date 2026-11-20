// The first-run explainer: DELAYED data, paper by default, and the
// disclaimer. On first run it can only be closed by acknowledging it
// (Enter on the focused button); WELCOME shows it again, and then Esc
// also closes it.

import { type Component, createEffect, Show } from "solid-js";
import { DISCLAIMER } from "../data/disclaimer";
import { finishOnboarding, inTauri } from "../ipc";
import { closeOnboarding, onboardingOpen } from "./onboardingState";
import "./Onboarding.css";

export const Onboarding: Component = () => {
  let button: HTMLButtonElement | undefined;
  createEffect(() => {
    if (onboardingOpen()) queueMicrotask(() => button?.focus());
  });

  const done = () => {
    const firstRun = onboardingOpen() === "first-run";
    closeOnboarding();
    // A failed write only means it shows again next start.
    if (firstRun && inTauri()) finishOnboarding().catch(() => {});
    queueMicrotask(() => document.querySelector<HTMLInputElement>(".cmdbar input")?.focus());
  };

  const onKeyDown = (e: KeyboardEvent) => {
    // Nothing behind the dialog takes keys while it is up.
    e.stopPropagation();
    if (e.key === "Escape" && onboardingOpen() === "again") {
      e.preventDefault();
      done();
    } else if (e.key === "Tab") {
      // One control: keep the focus on it.
      e.preventDefault();
      button?.focus();
    }
  };

  return (
    <Show when={onboardingOpen()}>
      <div class="onboarding" role="presentation">
        <div
          class="onboarding__box"
          role="dialog"
          aria-modal="true"
          aria-labelledby="onboarding-title"
          onKeyDown={onKeyDown}
        >
          <h2 id="onboarding-title">Welcome to Nivesh</h2>
          <section>
            <h3>Data is DELAYED unless it says LIVE</h3>
            <p>
              Without a broker connected, prices come from public sources, minutes behind the
              market. Every panel shows a DELAYED or STALE badge with its source and time. Live
              trading only ever acts on LIVE data.
            </p>
          </section>
          <section>
            <h3>Strategies trade on paper by default</h3>
            <p>
              Backtests and PAPER runs never send an order. Live trading is off until you turn it on
              with LIVE and arm a strategy with ARM, each confirmed by a typed phrase. The kill
              switch (Ctrl+Shift+K) stops everything at once.
            </p>
          </section>
          <section>
            <h3>Not advice</h3>
            <p>{DISCLAIMER}</p>
          </section>
          <p class="onboarding__keys">
            Type a symbol (RELIANCE) or a command (HELP) in the bar at the bottom. SETTINGS shows
            this again with WELCOME.
          </p>
          <button ref={button} type="button" class="onboarding__ok" onClick={done}>
            I understand
          </button>
        </div>
      </div>
    </Show>
  );
};
