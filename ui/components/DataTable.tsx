import {
  createEffect,
  createMemo,
  createSignal,
  For,
  type JSX,
  onCleanup,
  onMount,
  Show,
} from "solid-js";
import "./DataTable.css";
import { scrollToReveal, spacers, VIRTUALIZE_ABOVE, visibleWindow } from "./virtual";

export type CellTone = "up" | "down" | "dim" | "accent" | "stale";

export interface Column<Row> {
  id: string;
  header: string;
  /** Fixed column width, e.g. "12ch". Columns without one share the rest. */
  width?: string;
  align?: "left" | "right" | "center";
  value: (row: Row) => string | number;
  /** Custom cell content (a badge, say). Takes precedence over `value`. */
  render?: (row: Row) => JSX.Element;
  tone?: (row: Row) => CellTone | undefined;
}

export interface DataTableProps<Row> {
  /** Accessible name for the table. */
  label: string;
  columns: readonly Column<Row>[];
  rows: readonly Row[];
  selectedIndex?: number;
  /** Mouse selection. Keyboard selection is the owner's business. */
  onRowMouseDown?: (index: number) => void;
  emptyText?: string;
  /** Row height in px. Defaults to the `--row-h` token. Rows must all be this tall. */
  rowHeight?: number;
}

const DEFAULT_ROW_HEIGHT = 18;

function tokenRowHeight(): number {
  const raw = getComputedStyle(document.documentElement).getPropertyValue("--row-h");
  const n = Number.parseFloat(raw);
  return Number.isFinite(n) && n > 0 ? n : DEFAULT_ROW_HEIGHT;
}

/**
 * A dense table with a fixed row height (`--row-h`), so layout never depends
 * on content and long tables can be virtualised later without changing callers.
 */
export function DataTable<Row>(props: DataTableProps<Row>): JSX.Element {
  const cellClass = (column: Column<Row>, row?: Row) => {
    const tone = row === undefined ? undefined : column.tone?.(row);
    return `table__cell table__cell--${column.align ?? "left"}${tone ? ` ${tone}` : ""}`;
  };

  let table: HTMLTableElement | undefined;
  let body: HTMLTableSectionElement | undefined;

  // Long tables render only the rows near the viewport (the panel body is the
  // scroll container). Rows are a fixed height, so a row's position is its index.
  const [scroll, setScroll] = createSignal({ top: 0, viewport: 800, offset: 0 });
  let scroller: HTMLElement | undefined;
  const rowHeight = () => props.rowHeight ?? tokenRowHeight();
  const virtual = () => props.rows.length > VIRTUALIZE_ABOVE;

  const measure = () => {
    if (!scroller || !table) return;
    // Where the first body row sits inside the scrolled content.
    const offset =
      table.getBoundingClientRect().top - scroller.getBoundingClientRect().top + scroller.scrollTop;
    setScroll({
      top: scroller.scrollTop,
      viewport: scroller.clientHeight || 800,
      offset: offset + headerHeight(),
    });
  };
  const headerHeight = () => table?.tHead?.getBoundingClientRect().height ?? rowHeight();

  onMount(() => {
    scroller = table?.closest<HTMLElement>(".panel__body") ?? undefined;
    if (!scroller) return;
    measure();
    scroller.addEventListener("scroll", measure, { passive: true });
    const ro = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(measure);
    ro?.observe(scroller);
    onCleanup(() => {
      scroller?.removeEventListener("scroll", measure);
      ro?.disconnect();
    });
  });

  const win = createMemo(() => {
    const s = scroll();
    return visibleWindow({
      count: props.rows.length,
      rowHeight: rowHeight(),
      scrollTop: s.top,
      viewport: s.viewport,
      offsetTop: s.offset,
    });
  });
  const shown = createMemo(() =>
    virtual() ? props.rows.slice(win().start, win().end) : props.rows,
  );
  const first = () => (virtual() ? win().start : 0);
  const gap = () => spacers(win(), props.rows.length, rowHeight());

  // Keep the keyboard-selected row on screen. Rows have a fixed height, so the
  // row's position is simply its index, rendered or not.
  createEffect(() => {
    const i = props.selectedIndex;
    if (i === undefined) return;
    if (virtual() && scroller) {
      const s = scroll();
      const top = scrollToReveal(i, {
        rowHeight: rowHeight(),
        scrollTop: s.top,
        viewport: s.viewport,
        offsetTop: s.offset,
        headerHeight: headerHeight(),
      });
      if (top !== s.top) scroller.scrollTop = top;
      return;
    }
    body?.children[i]?.scrollIntoView?.({ block: "nearest" });
  });

  return (
    <table class="table" aria-label={props.label} ref={table}>
      <colgroup>
        <For each={props.columns}>
          {(column) => <col style={column.width ? { width: column.width } : undefined} />}
        </For>
      </colgroup>
      <thead>
        <tr class="table__row table__row--head">
          <For each={props.columns}>
            {(column) => (
              <th scope="col" class={cellClass(column)}>
                {column.header}
              </th>
            )}
          </For>
        </tr>
      </thead>
      <tbody ref={body}>
        <Show
          when={props.rows.length > 0}
          fallback={
            <tr class="table__row">
              <td class="table__empty" colSpan={props.columns.length}>
                {props.emptyText ?? "NO DATA"}
              </td>
            </tr>
          }
        >
          <Show when={virtual() && gap().before > 0}>
            <tr class="table__spacer" style={{ height: `${gap().before}px` }} />
          </Show>
          <For each={shown()}>
            {(row, i) => {
              const index = () => first() + i();
              return (
                <tr
                  class="table__row"
                  classList={{ "table__row--selected": index() === props.selectedIndex }}
                  aria-selected={index() === props.selectedIndex ? "true" : undefined}
                  aria-rowindex={virtual() ? index() + 2 : undefined}
                  onMouseDown={() => props.onRowMouseDown?.(index())}
                >
                  <For each={props.columns}>
                    {(column) => (
                      <td class={cellClass(column, row)}>
                        {column.render ? column.render(row) : column.value(row)}
                      </td>
                    )}
                  </For>
                </tr>
              );
            }}
          </For>
          <Show when={virtual() && gap().after > 0}>
            <tr class="table__spacer" style={{ height: `${gap().after}px` }} />
          </Show>
        </Show>
      </tbody>
    </table>
  );
}
