// The only module that touches Tauri's `invoke`. Components and stores call the
// typed wrappers in ui/ipc/*.ts, never `invoke` directly.
import { type InvokeArgs, invoke, isTauri } from "@tauri-apps/api/core";
import type { ErrorCode } from "./bindings/ErrorCode";
import type { IpcError } from "./bindings/IpcError";

/** A failed command. Carries the structured error from Rust. */
export class IpcFailure extends Error {
  readonly command: string;
  readonly code: ErrorCode;
  readonly retryable: boolean;

  constructor(command: string, error: IpcError) {
    super(error.message);
    this.name = "IpcFailure";
    this.command = command;
    this.code = error.code;
    this.retryable = error.retryable;
  }
}

const ERROR_CODES: ReadonlySet<string> = new Set<ErrorCode>([
  "INVALID_ARGUMENT",
  "NOT_FOUND",
  "UNAVAILABLE",
  "STORAGE",
  "INTERNAL",
]);

function isIpcError(value: unknown): value is IpcError {
  if (typeof value !== "object" || value === null) return false;
  const v = value as { code?: unknown; message?: unknown; retryable?: unknown };
  return (
    typeof v.code === "string" &&
    ERROR_CODES.has(v.code) &&
    typeof v.message === "string" &&
    typeof v.retryable === "boolean"
  );
}

/**
 * Normalises whatever `invoke` rejected with into an `IpcError`. Commands
 * return structured errors; Tauri itself rejects with plain strings (e.g. an
 * unknown command or a denied permission).
 */
export function toIpcError(reason: unknown): IpcError {
  if (isIpcError(reason)) return reason;
  const message =
    typeof reason === "string" ? reason : reason instanceof Error ? reason.message : String(reason);
  return { code: "INTERNAL", message, retryable: false };
}

/** Invokes a Rust command, rejecting with an `IpcFailure` on error. */
export async function call<T>(command: string, args?: InvokeArgs): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (reason) {
    throw new IpcFailure(command, toIpcError(reason));
  }
}

/** False when the UI runs outside the app (plain browser, unit tests). */
export function inTauri(): boolean {
  return isTauri();
}
