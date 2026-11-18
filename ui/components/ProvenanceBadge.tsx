import type { Component } from "solid-js";
import type { FreshnessDto, ProvenanceDto } from "../ipc";
import { formatAsOf } from "../lib/time";
import { Badge, type BadgeTone } from "./Badge";
import "./ProvenanceBadge.css";

/** The badge colour for each label. */
export const FRESHNESS_TONE: Record<FreshnessDto, BadgeTone> = {
  LIVE: "live",
  DELAYED: "delayed",
  EOD: "eod",
  STALE: "stale",
};

const SOURCE_NAMES: Record<string, string> = {
  nse: "NSE",
  yahoo: "YAHOO",
  kite: "KITE",
  rss: "RSS",
};

/** `nse` -> `NSE`. Unknown providers are shown upper-cased, never hidden. */
export function providerName(provider: string): string {
  return SOURCE_NAMES[provider] ?? provider.toUpperCase();
}

/** Explains the label in words, for the tooltip. */
export function freshnessTitle(p: ProvenanceDto, now?: number): string {
  const source = `${providerName(p.provider)}, as of ${formatAsOf(p.asOfMs, now)}`;
  switch (p.freshness) {
    case "LIVE":
      return `Live broker feed. ${source}`;
    case "DELAYED":
      return `Public data, unofficial and delayed. ${source}`;
    case "EOD":
      return `End-of-day data. ${source}`;
    case "STALE":
      return `Last known data: providers could not be reached. ${source}`;
  }
}

export interface ProvenanceBadgeProps {
  provenance: ProvenanceDto;
  /** Epoch ms used to decide whether the as-of time needs a date. Defaults to now. */
  now?: number;
}

/**
 * The one way market data is labelled: DELAYED / STALE (or EOD / LIVE), the
 * source and the as-of time. Public data is never LIVE; the backend sets the
 * label, this only displays it.
 */
export const ProvenanceBadge: Component<ProvenanceBadgeProps> = (props) => (
  <span class="prov" data-freshness={props.provenance.freshness}>
    <Badge
      tone={FRESHNESS_TONE[props.provenance.freshness]}
      title={freshnessTitle(props.provenance, props.now)}
    >
      {props.provenance.freshness}
    </Badge>
    <span class="prov__src">
      {providerName(props.provenance.provider)} {formatAsOf(props.provenance.asOfMs, props.now)}
    </span>
  </span>
);
