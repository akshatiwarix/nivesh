// First-run onboarding state. It opens once on a fresh install (the
// backend records when it was read) and again on WELCOME.

import { createSignal } from "solid-js";

const [open, setOpen] = createSignal<"first-run" | "again">();
const [startupMs, setStartupMs] = createSignal<number>();

export { open as onboardingOpen, setStartupMs, startupMs };

export function showOnboarding(firstRun: boolean): void {
  setOpen(firstRun ? "first-run" : "again");
}

export function closeOnboarding(): void {
  setOpen(undefined);
}
