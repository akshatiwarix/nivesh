// AUDIT: the hash-chained audit log, newest first, filtered by kind prefix
// and strategy, with the chain's verification status. Alt+Up/Down select a
// record; its full details show below the list. Read-only.

import { type Component, createMemo, createSignal, Show } from "solid-js";
import { Badge } from "../../components/Badge";
import { DataPanel } from "../../components/DataPanel";
import { type CellTone, type Column, DataTable } from "../../components/DataTable";
import { createPoll } from "../../data/poll";
import { type AuditRowDto, auditLog, auditVerify, inTauri } from "../../ipc";
import { formatIstDate, formatIstTime } from "../../lib/time";
import { usePanel, useViewOps } from "../context";
import type { View } from "../types";
import "./audit.css";

const AUDIT_REFRESH_MS = 5_000;
const VERIFY_REFRESH_MS = 30_000;

/** Kinds that mean trading stopped or something failed. */
export function kindTone(kind: string): CellTone | undefined {
  if (
    /^(kill\.engaged|risk\.rejected|order\.(failed|ambiguous|cancel_failed)|fill\.refused|intent\.(stale|not_live)|strategy\.(paused|blocked))$/.test(
      kind,
    )
  ) {
    return "down";
  }
  if (/^(live\.switch|strategy\.armed|limits\.changed|kill\.reset)$/.test(kind)) return "accent";
  return undefined;
}

const COLUMNS: readonly Column<AuditRowDto>[] = [
  { id: "seq", header: "#", width: "8ch", align: "right", value: (r) => String(r.seq) },
  {
    id: "at",
    header: "Time (IST)",
    width: "20ch",
    value: (r) => `${formatIstDate(r.at)} ${formatIstTime(r.at)}`,
  },
  {
    id: "kind",
    header: "Kind",
    width: "18ch",
    value: (r) => r.kind,
    tone: (r) => kindTone(r.kind),
  },
  {
    id: "strategy",
    header: "Strategy",
    width: "14ch",
    value: (r) => r.strategy ?? (r.strategyId === null ? "" : `#${r.strategyId}`),
  },
  { id: "payload", header: "Details", value: (r) => r.payload, tone: () => "dim" },
];

/** `AUDIT [KIND <prefix>] [FOR <NAME>]`. */
export const AuditView: Component<{ view: Extract<View, { kind: "audit" }> }> = (props) => {
  const panel = usePanel();
  const filter = () => ({ kind: props.view.filter, strategy: props.view.strategy });
  const rows = createPoll(
    () => (inTauri() ? filter() : false),
    (f) => auditLog(f),
    { everyMs: () => AUDIT_REFRESH_MS },
  );
  const chain = createPoll(
    () => inTauri(),
    () => auditVerify(),
    { everyMs: () => VERIFY_REFRESH_MS },
  );
  const list = createMemo(() => rows.data() ?? []);
  const [selected, setSelected] = createSignal(0);
  const current = () => list()[Math.min(selected(), list().length - 1)];

  useViewOps((op) => {
    const n = list().length;
    if (n === 0) return;
    if (op === "up") setSelected((i) => Math.max(0, Math.min(i, n - 1) - 1));
    if (op === "down") setSelected((i) => Math.min(n - 1, i + 1));
  });

  const filterText = () => {
    const parts = [];
    if (props.view.filter) parts.push(`kind ${props.view.filter}*`);
    if (props.view.strategy) parts.push(`strategy ${props.view.strategy}`);
    return parts.length ? parts.join(" · ") : "all records";
  };

  const emptyText = () => {
    if (!inTauri()) return "The audit log is available in the Nivesh app.";
    if (rows.error() && !rows.data()) return `${rows.error()}. Retrying.`;
    if (rows.data() && list().length === 0) return "No records match.";
    return undefined;
  };

  return (
    <div class="view audit" data-view="audit">
      <DataPanel provenance={null} title="AUDIT">
        <div class="audit__head">
          <Show when={chain.data()} fallback={<Badge tone="neutral">CHAIN UNCHECKED</Badge>}>
            {(v) => (
              <Show
                when={v().intact}
                fallback={
                  <Badge tone="down" title={v().reason ?? undefined}>
                    {`CHAIN BROKEN AT #${v().brokenAt}`}
                  </Badge>
                }
              >
                <Badge tone="up">{`CHAIN INTACT · ${v().rows} RECORDS`}</Badge>
              </Show>
            )}
          </Show>
          <span class="audit__filter">{filterText()}</span>
        </div>
        <Show when={chain.data() && !chain.data()?.intact}>
          <p class="view__error" role="alert">
            The audit chain is broken at record #{chain.data()?.brokenAt}: {chain.data()?.reason}.
            Records may have been changed or deleted.
          </p>
        </Show>
        <DataTable
          label="Audit records"
          columns={COLUMNS}
          rows={list()}
          selectedIndex={panel.focused() ? Math.min(selected(), list().length - 1) : undefined}
          onRowMouseDown={setSelected}
          emptyText={emptyText()}
        />
        <Show when={current()}>
          {(r) => (
            <pre class="audit__detail" title="Selected record">
              {`#${r().seq} ${r().kind}  hash ${r().hash.slice(0, 16)}…\n${r().payload}`}
            </pre>
          )}
        </Show>
        <p class="strat__hint">
          AUDIT [KIND &lt;PREFIX&gt;] [FOR &lt;NAME&gt;] · AUDIT VERIFY · AUDIT EXPORT CSV|JSONL ·
          Alt+Up/Down select · read-only: records can be exported, never edited.
        </p>
      </DataPanel>
    </div>
  );
};
