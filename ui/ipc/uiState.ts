import type { UiState } from "./bindings/UiState";
import { call } from "./core";

export function loadUiState(): Promise<UiState> {
  return call<UiState>("ui_state");
}

export function saveLayout(layout: 1 | 2 | 4): Promise<void> {
  return call<void>("save_layout", { layout });
}

export function saveCommandHistory(history: readonly string[]): Promise<void> {
  return call<void>("save_command_history", { history });
}

/** Records that the first-run onboarding was read. */
export function finishOnboarding(): Promise<void> {
  return call<void>("finish_onboarding");
}
