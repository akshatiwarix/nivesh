// Typed wrappers for the strategy commands.

import type { BacktestResultDto } from "./bindings/BacktestResultDto";
import type { StrategyDto } from "./bindings/StrategyDto";
import { call } from "./core";

export function strategies(): Promise<StrategyDto[]> {
  return call<StrategyDto[]>("strategies", {});
}

export function strategySave(args: {
  name: string;
  template: string;
  fast: number;
  slow: number;
  quantity: number;
  stopLoss?: number;
  target?: number;
}): Promise<number> {
  return call<number>("strategy_save", args);
}

export function strategyDelete(name: string): Promise<void> {
  return call<void>("strategy_delete", { name });
}

export function backtestStrategy(
  name: string,
  interval: string,
  range: string,
): Promise<BacktestResultDto> {
  return call<BacktestResultDto>("backtest_strategy", { name, interval, range });
}

export function backtestResult(name: string): Promise<BacktestResultDto> {
  return call<BacktestResultDto>("backtest_result", { name });
}

export function paperStart(name: string): Promise<unknown> {
  return call<unknown>("paper_start", { name });
}

export function paperStop(name: string): Promise<void> {
  return call<void>("paper_stop", { name });
}
