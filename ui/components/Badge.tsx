import type { ParentComponent } from "solid-js";
import "./Badge.css";

export type BadgeTone =
  | "live"
  | "delayed"
  | "eod"
  | "stale"
  | "paper"
  | "accent"
  | "up"
  | "down"
  | "neutral";

export interface BadgeProps {
  tone?: BadgeTone;
  /** Tooltip, e.g. source and as-of time. */
  title?: string;
}

/** A small upper-case label. Provenance labels must use the matching tone. */
export const Badge: ParentComponent<BadgeProps> = (props) => {
  return (
    <span class={`badge badge--${props.tone ?? "neutral"}`} title={props.title}>
      {props.children}
    </span>
  );
};
