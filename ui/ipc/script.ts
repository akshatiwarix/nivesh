// Typed wrappers for script strategies. Running them uses BT and PAPER.

import type { ScriptCheckDto } from "./bindings/ScriptCheckDto";
import { call } from "./core";

/** Compiles in the sandbox; nothing is saved. */
export function scriptValidate(source: string): Promise<ScriptCheckDto> {
  return call<ScriptCheckDto>("script_validate", { source });
}

/** Saves when it compiles (refused while the script is running). */
export function scriptSave(name: string, source: string): Promise<ScriptCheckDto> {
  return call<ScriptCheckDto>("script_save", { name, source });
}

/** The saved source, or null for a new name. */
export function scriptSource(name: string): Promise<string | null> {
  return call<string | null>("script_source", { name });
}
