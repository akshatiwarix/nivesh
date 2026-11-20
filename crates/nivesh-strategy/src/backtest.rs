//! The deterministic backtester: replays a strategy over a candle series,
//! bar by bar, with the same evaluator and the same fill model the paper
//! runner uses. **No lookahead**: the evaluator decides on closed bars and
//! a fill executes at the NEXT bar's open, never the signal bar itself.
//!
//! Progress and cancellation are surfaced to the IPC layer via a callback.

use nivesh_core::{Candle, Side};
use serde::{Deserialize, Serialize};

use crate::costs::CostModel;

use crate::decide::Decide;
use crate::evaluate::Evaluator;
use crate::rules::{IntentSide, OrderIntent, RuleSet};

/// One completed trade in the log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BacktestTrade {
    /// Bar index of the entry fill (the bar whose open filled it).
    pub entry_index: usize,
    pub exit_index: usize,
    /// UTC ms of the entry/exit fills.
    pub entry_t: i64,
    pub exit_t: i64,
    /// Fill prices (rupees): the next bar's open after slippage, long-only.
    pub entry_price: f64,
    pub exit_price: f64,
    pub quantity: i64,
    /// Charges on both fills, rupees.
    #[serde(default)]
    pub charges: f64,
    /// Realised P&L of the trade net of charges, rupees.
    pub pnl: f64,
}

/// The finished backtest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BacktestResult {
    pub trades: Vec<BacktestTrade>,
    /// The equity curve, one point per closed bar (rupees, start = equity).
    pub equity: Vec<f64>,
    /// Realised P&L over the whole run, net of charges.
    pub total_pnl: f64,
    /// Charges paid over the whole run, rupees.
    pub total_charges: f64,
    /// Max drawdown from a running peak (rupees, positive number).
    pub max_drawdown: f64,
    /// Bars in the market / total bars.
    pub bars_in_market: usize,
    pub total_bars: usize,
    /// Why the run stopped early, if it was cancelled.
    pub cancelled: bool,
}

/// One point of a run's equity curve.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    /// UTC ms of the bar the point is marked at (its close).
    pub t: i64,
    /// Equity marked at the bar's close, rupees.
    pub equity: f64,
    /// How far equity is below its running peak, rupees (zero or positive).
    pub drawdown: f64,
}

impl BacktestResult {
    /// The equity and drawdown curve, one point per traded bar. `bars` are
    /// the bars the run traded: all of them for [`backtest`], those from
    /// `trade_from` on for [`backtest_range`].
    pub fn curve(&self, bars: &[Candle]) -> Vec<CurvePoint> {
        // `equity[0]` is the starting equity; `equity[i + 1]` is after bar i.
        let mut peak = f64::NEG_INFINITY;
        bars.iter()
            .zip(self.equity.iter().skip(1))
            .map(|(bar, &equity)| {
                peak = peak.max(equity);
                CurvePoint {
                    t: bar.t,
                    equity,
                    drawdown: peak - equity,
                }
            })
            .collect()
    }
}

/// Options for a run.
#[derive(Debug, Clone, Copy)]
pub struct BacktestOptions {
    /// Starting paper equity, rupees.
    pub equity: f64,
    /// The symbol the candles belong to (intents carry it).
    pub symbol: &'static str,
    /// Charges and slippage on every fill.
    pub costs: CostModel,
}

impl Default for BacktestOptions {
    fn default() -> Self {
        Self {
            equity: 100_000.0,
            symbol: "BACKTEST",
            costs: CostModel::default(),
        }
    }
}

/// Runs the strategy over `candles` (oldest first). `progress(done, total)`
/// is called every 500 bars; returning `false` from `progress` cancels.
pub fn backtest(
    set: RuleSet,
    candles: &[Candle],
    options: BacktestOptions,
    progress: &mut dyn FnMut(usize, usize) -> bool,
) -> BacktestResult {
    backtest_range(set, candles, 0, options, progress)
}

/// Like [`backtest`], but trades only from `trade_from` on: the bars before
/// it warm the indicators and are never traded. A position still open at
/// the last bar is closed at its close, so a window's result stands alone.
/// Trade indices stay indices into `candles`.
pub fn backtest_range(
    set: RuleSet,
    candles: &[Candle],
    trade_from: usize,
    options: BacktestOptions,
    progress: &mut dyn FnMut(usize, usize) -> bool,
) -> BacktestResult {
    match Evaluator::new(set, options.equity) {
        Ok(e) => backtest_decider(
            Box::new(e.with_symbol(options.symbol)),
            candles,
            trade_from,
            options,
            progress,
        ),
        Err(_) => {
            let trade_from = trade_from.min(candles.len());
            BacktestResult {
                trades: Vec::new(),
                equity: vec![options.equity],
                total_pnl: 0.0,
                total_charges: 0.0,
                max_drawdown: 0.0,
                bars_in_market: 0,
                total_bars: candles.len() - trade_from,
                cancelled: false,
            }
        }
    }
}

/// Like [`backtest_range`], driving any [`Decide`]r: a rule evaluator or
/// a script. Same fills, same costs, same rules: no pyramiding, no exits
/// while flat, one bar of delay.
pub fn backtest_decider(
    mut evaluator: Box<dyn Decide>,
    candles: &[Candle],
    trade_from: usize,
    options: BacktestOptions,
    progress: &mut dyn FnMut(usize, usize) -> bool,
) -> BacktestResult {
    let trade_from = trade_from.min(candles.len());
    let mut trades: Vec<BacktestTrade> = Vec::new();
    let mut pending: Option<OrderIntent> = None;
    let mut equity_curve: Vec<f64> = Vec::with_capacity(candles.len() + 1);
    equity_curve.push(options.equity);
    let mut cash = options.equity;
    // (qty, entry price, entry index, entry t, entry charges)
    let mut position: Option<(i64, f64, usize, i64, f64)> = None;
    let mut total_charges = 0.0;
    let costs = options.costs;
    let mut bars_in_market = 0usize;

    // Warm-up: the indicators see these bars; nothing is traded on them.
    for bar in &candles[..trade_from] {
        let _ = evaluator.feed(bar);
        evaluator.sync_position(None);
    }

    let mut i = trade_from;
    while i < candles.len() {
        if i.is_multiple_of(500) && !progress(i, candles.len()) {
            let drawdown = max_drawdown(&equity_curve);
            return BacktestResult {
                trades,
                equity: equity_curve,
                total_pnl: cash - options.equity,
                total_charges,
                max_drawdown: drawdown,
                bars_in_market,
                total_bars: candles.len() - trade_from,
                cancelled: true,
            };
        }
        let bar = &candles[i];

        // 1. Fill a pending intent at THIS bar's open (the intent was made
        //    on the previous closed bar).
        if let Some(intent) = pending.take() {
            match intent.side {
                IntentSide::Buy if position.is_none() => {
                    let qty = intent.quantity.min(1_000_000);
                    if qty > 0 && bar.o > 0.0 {
                        let price = costs.fill_price(Side::Buy, bar.o);
                        let charges = costs.charges(Side::Buy, qty, price).total_rupees();
                        position = Some((qty, price, i, bar.t, charges));
                        cash -= price * qty as f64 + charges;
                        total_charges += charges;
                    }
                }
                IntentSide::Sell => {
                    if let Some((qty, entry_price, entry_index, entry_t, entry_charges)) =
                        position.take()
                    {
                        let exit_price = costs.fill_price(Side::Sell, bar.o);
                        let charges = costs.charges(Side::Sell, qty, exit_price).total_rupees();
                        cash += exit_price * qty as f64 - charges;
                        total_charges += charges;
                        trades.push(round_trip(
                            (entry_index, entry_t, entry_price, entry_charges),
                            (i, bar.t, exit_price, charges),
                            qty,
                        ));
                    }
                }
                _ => {}
            }
        }

        // 2. Feed the closed bar to the evaluator; its intent fills on the
        //    NEXT bar's open.
        if let Some(intent) = evaluator.feed(bar) {
            // An exit intent while flat is ignored; an entry while in a
            // position is ignored (no pyramiding in v1).
            let relevant = match intent.side {
                IntentSide::Buy => position.is_none(),
                IntentSide::Sell => position.is_some(),
            };
            if relevant {
                pending = Some(intent);
            }
        }

        // 3. Mark to market for the equity curve.
        let mark = match &position {
            Some((qty, _entry, _, _, _)) => cash + bar.c * *qty as f64,
            None => cash,
        };
        equity_curve.push(mark);
        if position.is_some() {
            bars_in_market += 1;
        }
        i += 1;
    }

    // Close any open position at the last close (the run's end).
    if let Some((qty, entry_price, entry_index, entry_t, entry_charges)) = position.take()
        && let Some(last) = candles.last()
    {
        let exit_price = costs.fill_price(Side::Sell, last.c);
        let charges = costs.charges(Side::Sell, qty, exit_price).total_rupees();
        cash += exit_price * qty as f64 - charges;
        total_charges += charges;
        trades.push(round_trip(
            (entry_index, entry_t, entry_price, entry_charges),
            (candles.len() - 1, last.t, exit_price, charges),
            qty,
        ));
        if let Some(equity) = equity_curve.last_mut() {
            *equity = cash;
        }
    }

    let total_pnl = cash - options.equity;
    BacktestResult {
        trades,
        max_drawdown: max_drawdown(&equity_curve),
        total_pnl,
        total_charges,
        equity: equity_curve,
        bars_in_market,
        total_bars: candles.len() - trade_from,
        cancelled: false,
    }
}

/// A completed trade from its two fills: (bar index, bar t, price,
/// charges) each.
fn round_trip(
    entry: (usize, i64, f64, f64),
    exit: (usize, i64, f64, f64),
    quantity: i64,
) -> BacktestTrade {
    let charges = entry.3 + exit.3;
    BacktestTrade {
        entry_index: entry.0,
        exit_index: exit.0,
        entry_t: entry.1,
        exit_t: exit.1,
        entry_price: entry.2,
        exit_price: exit.2,
        quantity,
        charges,
        pnl: (exit.2 - entry.2) * quantity as f64 - charges,
    }
}

fn max_drawdown(curve: &[f64]) -> f64 {
    let mut peak = f64::NEG_INFINITY;
    let mut worst: f64 = 0.0;
    for value in curve {
        peak = peak.max(*value);
        if peak > 0.0 {
            worst = worst.max(peak - value);
        }
    }
    worst
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{CrossDirection, Operand, Rule, Sizing};

    fn candle(t: i64, o: f64, c: f64) -> Candle {
        Candle {
            t,
            o,
            h: c.max(o),
            l: c.min(o),
            c,
            v: 100.0,
        }
    }

    fn crossing_set() -> RuleSet {
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

    #[test]
    fn a_backtest_fills_on_the_next_bar_open() {
        // Bars: 99, 100, 101 (crosses above on 3rd closed bar -> fill on the
        // 4th bar's open = 102), then 99 (exit signal) -> fill next open.
        let candles = vec![
            candle(1, 99.0, 99.0),
            candle(2, 100.0, 100.0),
            candle(3, 101.0, 101.0),
            candle(4, 102.0, 102.0),
            candle(5, 99.0, 99.0),
            candle(6, 98.0, 98.0),
        ];
        let mut progress = |_, _| true;
        let result = backtest(crossing_set(), &candles, free(), &mut progress);
        assert!(!result.cancelled);
        assert_eq!(result.trades.len(), 1);
        let trade = &result.trades[0];
        // Entry filled at bar index 3's open (102), exit at index 5's open (98).
        assert_eq!(trade.entry_price, 102.0);
        assert_eq!(trade.exit_price, 98.0);
        assert_eq!(trade.pnl, -4.0);
        assert_eq!(result.total_pnl, -4.0);
    }

    #[test]
    fn the_curve_marks_each_bar_and_measures_drawdown_from_the_peak() {
        let candles = vec![
            candle(1, 99.0, 99.0),
            candle(2, 100.0, 100.0),
            candle(3, 101.0, 101.0), // entry signal
            candle(4, 102.0, 104.0), // filled at 102, marked at 104
            candle(5, 99.0, 99.0),   // exit signal; marked at 99
            candle(6, 98.0, 98.0),   // sold at 98
        ];
        let mut progress = |_, _| true;
        let result = backtest(crossing_set(), &candles, free(), &mut progress);
        let curve = result.curve(&candles);
        assert_eq!(curve.len(), candles.len());
        let t: Vec<i64> = curve.iter().map(|p| p.t).collect();
        assert_eq!(t, vec![1, 2, 3, 4, 5, 6]);
        let equity: Vec<f64> = curve.iter().map(|p| p.equity - 100_000.0).collect();
        assert_eq!(equity, vec![0.0, 0.0, 0.0, 2.0, -3.0, -4.0]);
        let drawdown: Vec<f64> = curve.iter().map(|p| p.drawdown).collect();
        assert_eq!(drawdown, vec![0.0, 0.0, 0.0, 0.0, 5.0, 6.0]);
        assert_eq!(result.max_drawdown, 6.0);
    }

    fn free() -> BacktestOptions {
        BacktestOptions {
            costs: CostModel::FREE,
            ..BacktestOptions::default()
        }
    }

    #[test]
    fn costs_come_off_every_fill_and_the_pnl() {
        let candles = vec![
            candle(1, 99.0, 99.0),
            candle(2, 100.0, 100.0),
            candle(3, 101.0, 101.0),
            candle(4, 102.0, 102.0),
            candle(5, 99.0, 99.0),
            candle(6, 98.0, 98.0),
        ];
        let mut progress = |_, _| true;
        let options = BacktestOptions::default();
        let result = backtest(crossing_set(), &candles, options, &mut progress);
        let trade = &result.trades[0];
        // 5 bps of slippage against us on both fills.
        assert_eq!(trade.entry_price, 102.0 * 1.0005);
        assert_eq!(trade.exit_price, 98.0 * 0.9995);
        let buy = options.costs.charges(Side::Buy, 1, trade.entry_price);
        let sell = options.costs.charges(Side::Sell, 1, trade.exit_price);
        let charges = buy.total_rupees() + sell.total_rupees();
        assert_eq!(trade.charges, charges);
        assert_eq!(
            trade.pnl,
            (trade.exit_price - trade.entry_price) - charges,
            "net of slippage and charges"
        );
        assert_eq!(result.total_charges, charges);
        assert!((result.total_pnl - trade.pnl).abs() < 1e-9);
    }

    #[test]
    fn determinism_the_same_input_gives_the_same_output() {
        let candles: Vec<Candle> = (0..200)
            .map(|i| {
                candle(
                    i64::from(i) * 60_000,
                    100.0 + (i % 17) as f64,
                    100.0 + (i % 13) as f64,
                )
            })
            .collect();
        let mut progress = |_, _| true;
        let a = backtest(
            crossing_set(),
            &candles,
            BacktestOptions::default(),
            &mut progress,
        );
        let b = backtest(
            crossing_set(),
            &candles,
            BacktestOptions::default(),
            &mut progress,
        );
        assert_eq!(a, b, "byte-identical results for identical inputs");
    }

    #[test]
    fn no_lookahead_perturbing_future_bars_never_changes_past_decisions() {
        // Two series identical up to bar 40, different after. The trades
        // entered and exited before bar 40 must be identical.
        let base: Vec<Candle> = (0..60)
            .map(|i| {
                candle(
                    i64::from(i) * 60_000,
                    100.0 + (i % 11) as f64,
                    100.0 + (i % 9) as f64,
                )
            })
            .collect();
        let mut perturbed = base.clone();
        for bar in perturbed.iter_mut().skip(45) {
            bar.c += 1000.0;
            bar.o += 1000.0;
            bar.h += 1000.0;
            bar.l += 1000.0;
        }
        let mut progress = |_, _| true;
        let a = backtest(
            crossing_set(),
            &base,
            BacktestOptions::default(),
            &mut progress,
        );
        let b = backtest(
            crossing_set(),
            &perturbed,
            BacktestOptions::default(),
            &mut progress,
        );
        let early_a: Vec<_> = a.trades.iter().filter(|t| t.exit_index < 45).collect();
        let early_b: Vec<_> = b.trades.iter().filter(|t| t.exit_index < 45).collect();
        assert_eq!(early_a, early_b, "future bars changed past trades");
    }

    #[test]
    fn cancellation_stops_the_run() {
        let candles: Vec<Candle> = (0..10_000)
            .map(|i| candle(i64::from(i) * 60_000, 100.0, 100.0))
            .collect();
        let mut progress = |done, _| done < 2_000;
        let result = backtest(
            crossing_set(),
            &candles,
            BacktestOptions::default(),
            &mut progress,
        );
        assert!(result.cancelled);
        assert_eq!(result.total_bars, 10_000);
    }
}
