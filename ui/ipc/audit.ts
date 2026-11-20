// Typed wrappers for the audit log: read-only.

import type { AuditRowDto } from "./bindings/AuditRowDto";
import type { AuditVerifyDto } from "./bindings/AuditVerifyDto";
import { call } from "./core";

export interface AuditFilter {
  /** A kind prefix: `order`, `kill`, `strategy.armed`. */
  kind?: string;
  /** A strategy's name. */
  strategy?: string;
}

/** The newest records first (at most 5,000). */
export function auditLog(filter: AuditFilter = {}, limit?: number): Promise<AuditRowDto[]> {
  return call<AuditRowDto[]>("audit_log", {
    kind: filter.kind ?? null,
    strategy: filter.strategy ?? null,
    sinceMs: null,
    limit: limit ?? null,
  });
}

/** Re-checks the whole hash chain. */
export function auditVerify(): Promise<AuditVerifyDto> {
  return call<AuditVerifyDto>("audit_verify", {});
}

/** Writes the whole log to Downloads; resolves to the file's path. */
export function auditExport(format: "csv" | "jsonl"): Promise<string> {
  return call<string>("audit_export", { format });
}
