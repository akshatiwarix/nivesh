//! Out-of-sample and walk-forward checks: does the strategy hold up on
//! data it was not looked at on?
//!
//! Two views of one series, both built on [`backtest_range`] (each window
//! warms its indicators on the bars before it, trades only inside itself,
//! and closes any open position at its own end):
//!
//! - **In-sample / out-of-sample**: the first `in_sample` fraction of the
//!   bars versus the rest. A strategy whose edge is curve-fitted to the
//!   history usually loses it out of sample.
//! - **Walk-forward windows**: the series cut into `windows` consecutive,
//!   equal windows, each traded on its own, so one lucky stretch cannot
//!   carry the whole result.
//!
//! Deterministic like the backtester: the same inputs give identical
//! results.

use nivesh_core::Candle;
use serde::{Deserialize, Serialize};

use crate::backtest::{BacktestOptions, backtest_range};
use crate::rules::RuleSet;

/// How to split the series.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WalkForwardOptions {
    /// The in-sample share of the bars, `0.7` = the first 70%.
    pub in_sample: f64,
    /// How many consecutive walk-forward windows.
    pub windows: usize,
}

impl Default for WalkForwardOptions {
    fn default() -> Self {
        Self {
            in_sample: 0.7,
            windows: 5,
        }
    }
}

/// One window's result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowResult {
    /// The window's bars: indices `[start, end)` into the series, and the
    /// UTC ms of its first and last bar.
    pub start: usize,
    pub end: usize,
    pub start_t: i64,
    pub end_t: i64,
    pub trades: usize,
    pub winning_trades: usize,
    /// Net of charges, rupees.
    pub net_pnl: f64,
    pub charges: f64,
    pub max_drawdown: f64,
}

impl WindowResult {
    /// Winning trades over all trades, `None` without trades.
    pub fn win_rate(&self) -> Option<f64> {
        (self.trades > 0).then(|| self.winning_trades as f64 / self.trades as f64)
    }
}

/// The whole check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WalkForward {
    pub in_sample: WindowResult,
    pub out_of_sample: WindowResult,
    pub windows: Vec<WindowResult>,
    /// Windows that made money.
    pub profitable_windows: usize,
    /// The worst window's net P&L, rupees.
    pub worst_window_pnl: f64,
}

/// The `[start, end)` bounds of `windows` consecutive, near-equal windows
/// over `len` bars. Earlier windows take the remainder, one bar each.
pub fn window_bounds(len: usize, windows: usize) -> Vec<(usize, usize)> {
    let windows = windows.clamp(1, len.max(1));
    let (size, extra) = (len / windows, len % windows);
    let mut start = 0;
    (0..windows)
        .map(|w| {
            let end = start + size + usize::from(w < extra);
            let bounds = (start, end);
            start = end;
            bounds
        })
        .filter(|(s, e)| e > s)
        .collect()
}

/// Runs the in-sample/out-of-sample split and the walk-forward windows.
/// `None` when the series is too short to split (fewer than two bars per
/// window).
pub fn walk_forward(
    set: &RuleSet,
    candles: &[Candle],
    backtest: BacktestOptions,
    options: WalkForwardOptions,
) -> Option<WalkForward> {
    if candles.len() < 2 * options.windows.max(2) {
        return None;
    }
    let split = ((candles.len() as f64) * options.in_sample.clamp(0.05, 0.95)).floor() as usize;
    let split = split.clamp(1, candles.len() - 1);
    let in_sample = window(set, candles, 0, split, backtest);
    let out_of_sample = window(set, candles, split, candles.len(), backtest);
    let windows: Vec<WindowResult> = window_bounds(candles.len(), options.windows)
        .into_iter()
        .map(|(start, end)| window(set, candles, start, end, backtest))
        .collect();
    let profitable_windows = windows.iter().filter(|w| w.net_pnl > 0.0).count();
    let worst_window_pnl = windows
        .iter()
        .map(|w| w.net_pnl)
        .fold(f64::INFINITY, f64::min);
    Some(WalkForward {
        in_sample,
        out_of_sample,
        windows,
        profitable_windows,
        worst_window_pnl,
    })
}

fn window(
    set: &RuleSet,
    candles: &[Candle],
    start: usize,
    end: usize,
    options: BacktestOptions,
) -> WindowResult {
    let mut progress = |_, _| true;
    let result = backtest_range(set.clone(), &candles[..end], start, options, &mut progress);
    WindowResult {
        start,
        end,
        start_t: candles[start].t,
        end_t: candles[end - 1].t,
        trades: result.trades.len(),
        winning_trades: result.trades.iter().filter(|t| t.pnl > 0.0).count(),
        net_pnl: result.total_pnl,
        charges: result.total_charges,
        max_drawdown: result.max_drawdown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backtest::backtest;
    use crate::costs::CostModel;
    use crate::rules::{CrossDirection, Operand, Rule, Sizing};

    #[test]
    fn windows_cover_the_series_without_gaps() {
        assert_eq!(
            window_bounds(100, 5),
            vec![(0, 20), (20, 40), (40, 60), (60, 80), (80, 100)]
        );
        // The remainder goes to the earliest windows.
        assert_eq!(window_bounds(7, 3), vec![(0, 3), (3, 5), (5, 7)]);
        assert_eq!(window_bounds(3, 10), vec![(0, 1), (1, 2), (2, 3)]);
    }

    fn candle(i: usize, c: f64) -> Candle {
        Candle {
            t: 1_764_000_000_000 + i as i64 * 86_400_000,
            o: c,
            h: c + 1.0,
            l: c - 1.0,
            c,
            v: 100.0,
        }
    }

    /// Long above 100, out below it.
    fn level_set() -> RuleSet {
        RuleSet {
            entry: Rule::Cross {
                left: Operand::Close,
                direction: CrossDirection::Above,
                right: Operand::Constant(100.0),
            },
            exit: Rule::Cross {
                left: Operand::Close,
                direction: CrossDirection::Below,
                right: Operand::Constant(100.0),
            },
            sizing: Sizing::FixedQuantity(1),
            ..RuleSet::default()
        }
    }

    fn free() -> BacktestOptions {
        BacktestOptions {
            costs: CostModel::FREE,
            ..BacktestOptions::default()
        }
    }

    #[test]
    fn a_window_only_trades_inside_itself() {
        // Crosses above 100 at bar 2 (fill at 3's open) and back below at
        // bar 6 (fill at 7's open); then above again at bar 9 (fill at 10's).
        let closes = [
            99.0, 99.0, 101.0, 102.0, 104.0, 103.0, 98.0, 97.0, 99.0, 105.0, 106.0,
        ];
        let bars: Vec<Candle> = closes
            .iter()
            .enumerate()
            .map(|(i, c)| candle(i, *c))
            .collect();
        let mut progress = |_, _| true;
        // Trading only from bar 5: the bar-2 cross happened in the warm-up,
        // so the window starts flat and only the bar-9 entry is live; it is
        // force-closed at the window's (series') last close.
        let late = backtest_range(level_set(), &bars, 5, free(), &mut progress);
        assert_eq!(late.trades.len(), 1);
        assert_eq!(late.trades[0].entry_index, 10);
        assert_eq!(late.total_bars, 6);
        // From bar 0 it is the plain backtest.
        assert_eq!(
            backtest_range(level_set(), &bars, 0, free(), &mut progress),
            backtest(level_set(), &bars, free(), &mut progress)
        );
    }

    #[test]
    fn the_split_and_the_windows_have_the_right_bounds_and_metrics() {
        // 20 bars, each opening at its close, with one losing round trip in
        // each half. Fills are at the next bar's open.
        let mut closes = [99.0; 20];
        closes[2] = 101.0; // crosses above: buy at bar 3's open, 105
        closes[3] = 105.0;
        closes[4] = 106.0;
        closes[5] = 98.0; // crosses below: sell at bar 6's open, 99
        closes[12] = 101.0; // crosses above: buy at bar 13's open, 99.5
        closes[13] = 99.5; // crosses below: sell at bar 14's open, 99
        let bars: Vec<Candle> = closes
            .iter()
            .enumerate()
            .map(|(i, c)| candle(i, *c))
            .collect();
        let wf = walk_forward(
            &level_set(),
            &bars,
            free(),
            WalkForwardOptions {
                in_sample: 0.5,
                windows: 2,
            },
        )
        .unwrap();
        assert_eq!((wf.in_sample.start, wf.in_sample.end), (0, 10));
        assert_eq!((wf.out_of_sample.start, wf.out_of_sample.end), (10, 20));
        assert_eq!(wf.in_sample.start_t, bars[0].t);
        assert_eq!(wf.out_of_sample.end_t, bars[19].t);
        assert_eq!(wf.in_sample.trades, 1);
        assert_eq!(wf.in_sample.net_pnl, 99.0 - 105.0);
        assert_eq!(wf.in_sample.win_rate(), Some(0.0));
        assert_eq!(wf.out_of_sample.trades, 1);
        assert_eq!(wf.out_of_sample.net_pnl, 99.0 - 99.5);
        assert_eq!(wf.windows.len(), 2);
        assert_eq!(wf.windows[0], wf.in_sample, "same bounds, same result");
        assert_eq!(wf.profitable_windows, 0);
        assert_eq!(wf.worst_window_pnl, -6.0);
    }

    #[test]
    fn too_short_a_series_is_not_split() {
        let bars: Vec<Candle> = (0..5).map(|i| candle(i, 100.0)).collect();
        assert!(walk_forward(&level_set(), &bars, free(), WalkForwardOptions::default()).is_none());
    }

    #[test]
    fn walk_forward_is_deterministic() {
        let bars: Vec<Candle> = (0..300)
            .map(|i| candle(i, 100.0 + ((i % 23) as f64 - 11.0)))
            .collect();
        let options = WalkForwardOptions::default();
        let a = walk_forward(&level_set(), &bars, BacktestOptions::default(), options);
        let b = walk_forward(&level_set(), &bars, BacktestOptions::default(), options);
        assert!(a.is_some());
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap(),
            "byte-identical"
        );
    }
}
