// The state of BT runs started from the command bar, so the BT view shows
// "Running…", then the new result (or why it failed) instead of whatever
// was saved before the run.

import { createSignal } from "solid-js";

export type BtRun =
  | { state: "running" }
  | { state: "done"; at: number }
  | { state: "failed"; error: string };

const [runs, setRuns] = createSignal<Readonly<Record<string, BtRun>>>({});

const key = (name: string) => name.toUpperCase();

export function btRun(name: string | undefined): BtRun | undefined {
  return name === undefined ? undefined : runs()[key(name)];
}

export function setBtRun(name: string, run: BtRun): void {
  setRuns((all) => ({ ...all, [key(name)]: run }));
}
