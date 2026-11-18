import type { Component } from "solid-js";

export const EmptyView: Component = () => (
  <div class="view" data-view="empty">
    <p class="view__note">Type a command, e.g. RELIANCE or NIFTY.</p>
  </div>
);
