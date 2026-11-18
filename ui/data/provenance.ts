// Summarising the provenance of a view that shows several values.
import type { FreshnessDto, ProvenanceDto } from "../ipc";

/** Worst first: what a mixed view should be labelled as. */
const SEVERITY: readonly FreshnessDto[] = ["STALE", "DELAYED", "EOD", "LIVE"];

export interface ProvenanceSummary {
  /** The label for the whole view: the least fresh of its parts. */
  freshness: FreshnessDto;
  /** Distinct providers, in first-seen order. */
  providers: string[];
  /** The oldest as-of time: the view is only as current as its stalest value. */
  oldestAsOfMs: number;
}

/** Summarises one or several provenances. `undefined` for none. */
export function summarizeProvenance(
  input: ProvenanceDto | readonly ProvenanceDto[] | null | undefined,
): ProvenanceSummary | undefined {
  const list = input === null || input === undefined ? [] : "provider" in input ? [input] : input;
  const first = list[0];
  if (first === undefined) return undefined;
  const providers: string[] = [];
  let worst = SEVERITY.length - 1;
  let oldest = first.asOfMs;
  for (const p of list) {
    if (!providers.includes(p.provider)) providers.push(p.provider);
    worst = Math.min(worst, SEVERITY.indexOf(p.freshness));
    oldest = Math.min(oldest, p.asOfMs);
  }
  return { freshness: SEVERITY[worst] ?? "STALE", providers, oldestAsOfMs: oldest };
}
