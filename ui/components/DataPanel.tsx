import {
  createEffect,
  createMemo,
  createSignal,
  type JSX,
  type ParentComponent,
  Show,
} from "solid-js";
import { summarizeProvenance } from "../data/provenance";
import type { ProvenanceDto } from "../ipc";
import { useNow } from "../lib/now";
import { formatAge, formatAsOf } from "../lib/time";
import { Badge } from "./Badge";
import "./DataPanel.css";
import { FRESHNESS_TONE, providerName } from "./ProvenanceBadge";

/**
 * What a view's data came from. Required, so a view cannot show market data
 * without saying where it is from: pass `null` while there is none yet.
 */
export type ProvenanceInput = ProvenanceDto | readonly ProvenanceDto[] | null;

export interface DataPanelProps {
  provenance: ProvenanceInput;
  /** Left side of the bar, e.g. the instrument's name. */
  title?: JSX.Element;
  /** Epoch ms used for ages and "today" decisions. Defaults to a shared ticking clock. */
  now?: () => number;
}

const sourcesText = (providers: readonly string[]) => providers.map(providerName).join(" + ");

/**
 * The frame around market data: a bar with the DELAYED / STALE label, source,
 * as-of time and age, a banner when the data is stale, and a note if the source
 * changed while the view was open (for instance after a fallback).
 */
export const DataPanel: ParentComponent<DataPanelProps> = (props) => {
  const clock = useNow();
  const now = () => props.now?.() ?? clock();
  const summary = createMemo(() => summarizeProvenance(props.provenance));

  // Remember a change of source for as long as the view stays open.
  const [changed, setChanged] = createSignal<string>();
  let previous: string[] | undefined;
  createEffect(() => {
    const providers = summary()?.providers;
    if (!providers) return;
    if (previous && previous.join() !== providers.join()) {
      setChanged(`${sourcesText(previous)} → ${sourcesText(providers)}`);
    }
    previous = providers;
  });

  return (
    <div class="datapanel" data-freshness={summary()?.freshness ?? "NONE"}>
      <div class="datapanel__bar">
        <span class="datapanel__title">{props.title}</span>
        <Show when={summary()} fallback={<Badge tone="neutral">NO DATA</Badge>}>
          {(s) => (
            <>
              <Badge
                tone={FRESHNESS_TONE[s().freshness]}
                title={
                  s().freshness === "STALE"
                    ? "Last known data: providers could not be reached."
                    : "Public data: unofficial and delayed."
                }
              >
                {s().freshness}
              </Badge>
              <span class="datapanel__src">
                {sourcesText(s().providers)} {formatAsOf(s().oldestAsOfMs, now())}
              </span>
              <span class="datapanel__age">{formatAge(now() - s().oldestAsOfMs)} ago</span>
            </>
          )}
        </Show>
      </div>
      <Show when={summary()?.freshness === "STALE"}>
        <p class="datapanel__note datapanel__note--stale" role="status">
          STALE: showing the last known data because the providers could not be reached.
        </p>
      </Show>
      <Show when={changed()}>
        {(c) => (
          <p class="datapanel__note" role="status">
            Source changed: {c()}
          </p>
        )}
      </Show>
      {props.children}
    </div>
  );
};
