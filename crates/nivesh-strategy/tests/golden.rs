//! Golden backtests: fixed datasets with `insta` snapshots for each
//! template, so a change in evaluator or fill semantics shows up as a
//! reviewed diff. Alongside the no-lookahead property test (below), these
//! are the plan's Part 5 acceptance pieces.

use nivesh_core::Candle;
use nivesh_strategy::{BacktestOptions, Template, TemplateParams, backtest};

/// A synthetic day series: a sawtooth around a drift, deterministic.
fn series(n: usize) -> Vec<Candle> {
    (0..n)
        .map(|i| {
            let t = 1_764_000_000_000 + i64::from(i as u32) * 86_400_000;
            let wave = f64::sin(i as f64 / 5.0) * 40.0;
            let drift = f64::from(i as u32 % 30);
            let close = 1_000.0 + wave + drift;
            Candle {
                t,
                o: close - 1.5,
                h: close + 2.0,
                l: close - 3.0,
                c: close,
                v: 10_000.0,
            }
        })
        .collect()
}

fn run(template: Template, n: usize) -> String {
    let set = template.build(&TemplateParams::default());
    let candles = series(n);
    let mut progress = |_, _| true;
    let result = backtest(set, &candles, BacktestOptions::default(), &mut progress);
    format!(
        "{}\ntrades: {}\ntotal_pnl: {:.2}\nmax_drawdown: {:.2}\nbars_in_market: {}\n",
        template.title(),
        result.trades.len(),
        result.total_pnl,
        result.max_drawdown,
        result.bars_in_market,
    )
}

#[test]
fn ma_crossover_golden() {
    insta::assert_snapshot!(run(Template::MaCrossover, 400), @r###"
MA Crossover
trades: 11
total_pnl: -355.30
max_drawdown: 356.80
bars_in_market: 42

"###);
}

#[test]
fn rsi_reversion_golden() {
    insta::assert_snapshot!(run(Template::RsiReversion, 400), @r###"
RSI Mean Reversion
trades: 5
total_pnl: 299.10
max_drawdown: 0.00
bars_in_market: 44

"###);
}

#[test]
fn breakout_golden() {
    insta::assert_snapshot!(run(Template::Breakout, 400), @r###"
N-Bar Breakout
trades: 13
total_pnl: 367.70
max_drawdown: 69.76
bars_in_market: 115

"###);
}
