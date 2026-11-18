import { For, type JSX, Show } from "solid-js";
import "./DataTable.css";

export type CellTone = "up" | "down" | "dim" | "accent";

export interface Column<Row> {
  id: string;
  header: string;
  /** Fixed column width, e.g. "12ch". Columns without one share the rest. */
  width?: string;
  align?: "left" | "right" | "center";
  value: (row: Row) => string | number;
  tone?: (row: Row) => CellTone | undefined;
}

export interface DataTableProps<Row> {
  /** Accessible name for the table. */
  label: string;
  columns: readonly Column<Row>[];
  rows: readonly Row[];
  selectedIndex?: number;
  emptyText?: string;
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

  return (
    <table class="table" aria-label={props.label}>
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
      <tbody>
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
          <For each={props.rows}>
            {(row, index) => (
              <tr
                class="table__row"
                classList={{ "table__row--selected": index() === props.selectedIndex }}
              >
                <For each={props.columns}>
                  {(column) => <td class={cellClass(column, row)}>{column.value(row)}</td>}
                </For>
              </tr>
            )}
          </For>
        </Show>
      </tbody>
    </table>
  );
}
