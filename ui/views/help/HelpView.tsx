import type { Component } from "solid-js";
import { DataTable } from "../../components/DataTable";
import { commandRows, type HelpRow, keyRows, targetRows } from "./rows";

const COLUMNS = [
  {
    id: "keys",
    header: "INPUT",
    width: "34ch",
    value: (r: HelpRow) => r.keys,
    tone: () => "accent" as const,
  },
  { id: "summary", header: "DOES", value: (r: HelpRow) => r.summary },
];

/** HELP: renders the command registry and the keymap. */
export const HelpView: Component = () => (
  <div class="view" data-view="help">
    <h2 class="view__title">Commands</h2>
    <DataTable label="Commands" columns={COLUMNS} rows={commandRows()} />
    <h2 class="view__title">Targets</h2>
    <DataTable label="Targets" columns={COLUMNS} rows={targetRows()} />
    <h2 class="view__title">Keys</h2>
    <DataTable label="Keys" columns={COLUMNS} rows={keyRows()} />
  </div>
);
