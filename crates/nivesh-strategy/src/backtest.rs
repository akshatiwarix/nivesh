//! The deterministic backtester: replays a strategy over a candle series,
//! bar by bar, with the same evaluator and the same fill model the paper
//! runner uses. **No lookahead**: the evaluator decides on closed bars and
//! a fill executes at the NEXT bar's open, never the signal bar itself.
//!
//! Progress and cancellation are surfaced to the IPC layer via a callback.

use nivesh_core::Candle;

use crate::evaluate::Evaluator;
use crate::rules::{IntentSide, OrderIntent, RuleSet};

/// One completed trade in the log.
#[derive(Debug, Clone, PartialEq)]
pub struct BacktestTrade {
    /// Bar index of the entry fill (the bar whose open filled it).
    pub entry_index: usize,
    pub exit_index: usize,
    /// UTC ms of the entry/exit fills.
    pub entry_t: i64,
    pub exit_t: i64,
    /// Fill prices (rupees): the next bar's open, long-only.
    pub entry_price: f64,
    pub exit_price: f64,
    pub quantity: i64,
    /// Realised P&L of the trade (rupees, fees ignored in v1 — the cost
    /// model lands with the paper runner where the playbook requires it).
    pub pnl: f64,
}

/// The finished backtest.
#[derive(Debug, Clone, PartialEq)]
pub struct BacktestResult {
    pub trades: Vec<BacktestTrade>,
    /// The equity curve, one point per closed bar (rupees, start = equity).
    pub equity: Vec<f64>,
    /// Realised P&L over the whole run.
    pub total_pnl: f64,
    /// Max drawdown from a running peak (rupees, positive number).
    pub max_drawdown: f64,
    /// Bars in the market / total bars.
    pub bars_in_market: usize,
    pub total_bars: usize,
    /// Why the run stopped early, if it was cancelled.
    pub cancelled: bool,
}

/// Options for a run.
#[derive(Debug, Clone, Copy)]
pub struct BacktestOptions {
    /// Starting paper equity, rupees.
    pub equity: f64,
    /// The symbol the candles belong to (intents carry it).
    pub symbol: &'static str,
}

impl Default for BacktestOptions {
    fn default() -> Self {
        Self {
            equity: 100_000.0,
            symbol: "BACKTEST",
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
    let mut evaluator = match Evaluator::new(set, options.equity) {
        Ok(e) => e.with_symbol(options.symbol),
        Err(_) => {
            return BacktestResult {
                trades: Vec::new(),
                equity: vec![options.equity],
                total_pnl: 0.0,
                max_drawdown: 0.0,
                bars_in_market: 0,
                total_bars: candles.len(),
                cancelled: false,
            };
        }
    };

    let mut trades: Vec<BacktestTrade> = Vec::new();
    let mut pending: Option<OrderIntent> = None;
    let mut equity_curve: Vec<f64> = Vec::with_capacity(candles.len() + 1);
    equity_curve.push(options.equity);
    let mut cash = options.equity;
    let mut position: Option<(i64, f64, usize, i64)> = None; // (qty, entry price, entry index, entry t)
    let mut bars_in_market = 0usize;

    let mut i = 0usize;
    while i < candles.len() {
        if i.is_multiple_of(500) && !progress(i, candles.len()) {
            let drawdown = max_drawdown(&equity_curve);
            return BacktestResult {
                trades,
                equity: equity_curve,
                total_pnl: cash - options.equity,
                max_drawdown: drawdown,
                bars_in_market,
                total_bars: candles.len(),
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
                        position = Some((qty, bar.o, i, bar.t));
                        cash -= bar.o * qty as f64;
                    }
                }
                IntentSide::Sell => {
                    if let Some((qty, entry_price, entry_index, entry_t)) = position.take() {
                        let exit_price = bar.o;
                        cash += exit_price * qty as f64;
                        trades.push(BacktestTrade {
                            entry_index,
                            exit_index: i,
                            entry_t,
                            exit_t: bar.t,
                            entry_price,
                            exit_price,
                            quantity: qty,
                            pnl: (exit_price - entry_price) * qty as f64,
                        });
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
            Some((qty, _entry, _, _)) => cash + bar.c * *qty as f64,
            None => cash,
        };
        equity_curve.push(mark);
        if position.is_some() {
            bars_in_market += 1;
        }
        i += 1;
    }

    // Close any open position at the last close (the run's end).
    if let Some((qty, entry_price, entry_index, entry_t)) = position.take()
        && let Some(last) = candles.last()
    {
        cash += last.c * qty as f64;
        trades.push(BacktestTrade {
            entry_index,
            exit_index: candles.len() - 1,
            entry_t,
            exit_t: last.t,
            entry_price,
            exit_price: last.c,
            quantity: qty,
            pnl: (last.c - entry_price) * qty as f64,
        });
        if let Some(equity) = equity_curve.last_mut() {
            *equity = cash;
        }
    }

    let total_pnl = cash - options.equity;
    BacktestResult {
        trades,
        max_drawdown: max_drawdown(&equity_curve),
        total_pnl,
        equity: equity_curve,
        bars_in_market,
        total_bars: candles.len(),
        cancelled: false,
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
        let result = backtest(
            crossing_set(),
            &candles,
            BacktestOptions::default(),
            &mut progress,
        );
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
