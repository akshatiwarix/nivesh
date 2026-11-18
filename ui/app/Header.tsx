import type { Component, JSX } from "solid-js";
import { formatIstDate, formatIstTime } from "../lib/time";

export interface HeaderProps {
  /** Current time, epoch milliseconds. */
  now: number;
  /** Market-status indicator; filled in once the backend reports a session. */
  status?: JSX.Element;
}

/** Top bar: product name, market-status slot and the IST clock. */
export const Header: Component<HeaderProps> = (props) => (
  <header class="shell__header">
    <span class="shell__brand">NIVESH</span>
    <span class="shell__status">{props.status}</span>
    <span class="shell__clock">
      <span>{formatIstDate(props.now)}</span>
      <span class="shell__time">{formatIstTime(props.now)} IST</span>
    </span>
  </header>
);
