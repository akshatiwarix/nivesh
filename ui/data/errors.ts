import { IpcFailure } from "../ipc";

/** A message fit for the command bar or an inline error line. */
export function describeError(err: unknown): string {
  return err instanceof IpcFailure || err instanceof Error ? err.message : String(err);
}
