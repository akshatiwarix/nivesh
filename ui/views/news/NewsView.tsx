import { type Component, createMemo, createSignal, Show } from "solid-js";
import { DataPanel } from "../../components/DataPanel";
import { type Column, DataTable } from "../../components/DataTable";
import { describeError } from "../../data/errors";
import { createPoll } from "../../data/poll";
import type { NewsItemDto } from "../../ipc";
import { getNews, inTauri, openNewsLink } from "../../ipc";
import { formatIstDate, formatIstHm } from "../../lib/time";
import { useViewOps } from "../context";
import { createSelection } from "../selection";
import type { View } from "../types";
import "./NewsView.css";

const REFRESH_MS = 60_000;
const LIMIT = 100;

/** `15:42` for today's headlines and `17 NOV 15:42` for older ones (IST). */
export function newsTime(publishedMs: number, nowMs: number = Date.now()): string {
  const day = (ms: number) => Math.floor((ms + 330 * 60_000) / 86_400_000);
  if (day(publishedMs) === day(nowMs)) return formatIstHm(publishedMs);
  const [, d, mon] = formatIstDate(publishedMs).split(" ");
  return `${d} ${mon} ${formatIstHm(publishedMs)}`;
}

const COLUMNS: readonly Column<NewsItemDto>[] = [
  {
    id: "time",
    header: "Time (IST)",
    width: "14ch",
    value: (n) => newsTime(n.publishedMs),
    tone: () => "dim",
  },
  { id: "source", header: "Source", width: "18ch", value: (n) => n.source, tone: () => "dim" },
  { id: "title", header: "Headline", value: (n) => n.title },
  {
    id: "tags",
    header: "Symbols",
    width: "22ch",
    value: (n) => n.tags.join(" "),
    tone: () => "accent",
  },
];

/**
 * `NEWS` / F7, or `RELIANCE NEWS`: headlines from public RSS feeds. Text only:
 * nothing from a feed is rendered as HTML or loaded in the app. Enter opens the
 * selected headline's link in the system browser (http/https links from the
 * feed only).
 */
export const NewsView: Component<{ view: Extract<View, { kind: "news" }> }> = (props) => {
  const [note, setNote] = createSignal<string>();
  const poll = createPoll(
    () => (inTauri() ? (props.view.symbol ?? "") : false),
    (symbol) => getNews(symbol === "" ? undefined : symbol, LIMIT),
    { everyMs: () => REFRESH_MS },
  );
  const rows = createMemo(() => poll.data()?.items ?? []);
  const selection = createSelection(() => rows().length);

  const open = async () => {
    const item = rows()[selection.index()];
    if (!item) return;
    try {
      await openNewsLink(item.url);
      setNote(`Opened in your browser: ${item.source}`);
    } catch (err) {
      setNote(`Could not open the link: ${describeError(err)}`);
    }
  };

  useViewOps((op) => {
    if (op === "up") selection.move(-1);
    else if (op === "down") selection.move(1);
    else if (op === "open" || op === "openBeside") void open();
  });

  const title = () => (props.view.symbol ? `NEWS · ${props.view.symbol}` : "NEWS");
  const emptyText = () => {
    if (!inTauri()) return "News is available in the Nivesh app.";
    if (poll.error() && !poll.data()) return `${poll.error()}. Retrying.`;
    if (!poll.data()) return "Loading…";
    return props.view.symbol
      ? `No recent headlines mention ${props.view.symbol}.`
      : "No headlines right now.";
  };

  return (
    <div class="view news" data-view="news">
      <DataPanel provenance={poll.data()?.provenance ?? null} title={title()}>
        <Show when={poll.data() && poll.error()}>
          <p class="view__error" role="alert">
            Refresh failed: {poll.error()}. Showing the last headlines.
          </p>
        </Show>
        <Show when={note()}>
          {(text) => (
            <p class="news__note" role="status">
              {text()}
            </p>
          )}
        </Show>
        <DataTable
          label={title()}
          columns={COLUMNS}
          rows={rows()}
          selectedIndex={selection.index()}
          onRowMouseDown={selection.set}
          emptyText={emptyText()}
        />
        <p class="news__hint">Enter opens the link in your browser. Headlines and sources only.</p>
      </DataPanel>
    </div>
  );
};
