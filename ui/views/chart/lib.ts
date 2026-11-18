// The chart library is large (about 45 KB gzipped), so it is only fetched when
// a chart is first opened. This is the one place that imports it at runtime.
export type ChartLib = typeof import("lightweight-charts");

let loading: Promise<ChartLib> | undefined;

export function loadChartLib(): Promise<ChartLib> {
  loading ??= import("lightweight-charts");
  return loading;
}
