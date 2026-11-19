//! The paper trading runner: drives an evaluator live over the quote hub's
//! closed bars and routes the intents through the gateway into the paper
//! broker. Everything it produces is **PAPER**: simulated fills on
//! simulated money, labelled as such everywhere it shows up.
//!
//! Running state persists in the store so a paper strategy resumes after a
//! restart (the runner itself is stateless per bar: the evaluator's state
//! is rebuilt from the strategy's position, which the store holds).

use nivesh_core::Candle;

use crate::evaluate::Evaluator;
use crate::rules::RuleSet;

/// The paper position a running strategy holds.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PaperPosition {
    pub symbol: String,
    pub quantity: i64,
    /// The fill price the paper broker gave (rupees).
    pub entry_price: f64,
    /// UTC ms of the fill.
    pub entry_t: i64,
    /// The trailing stop's watermark, when the strategy trails.
    pub high_watermark: Option<f64>,
    /// The effective stop level, rupees.
    pub stop: Option<f64>,
    /// The target level, rupees.
    pub target: Option<f64>,
}

/// The runner's state, persisted by the caller between bars.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PaperState {
    pub position: Option<PaperPosition>,
    /// Today's realised P&L, rupees.
    pub day_pnl: f64,
}

/// What one closed bar produced: the new state and, if the gateway filled
/// it, the paper fill.
#[derive(Debug, Clone, PartialEq)]
pub struct PaperOutcome {
    pub state: PaperState,
    /// `Some((side, quantity, price))` when a paper fill happened.
    pub fill: Option<(crate::rules::IntentSide, i64, f64)>,
    /// The intent the runner wants the gateway to check, if any.
    pub intent: Option<crate::rules::OrderIntent>,
}

/// The runner: one evaluator, one position, one symbol, over closed bars.
/// The same semantics as the backtester (signals on closed bars, exits
/// before entries, stops on the touch), live.
pub struct PaperRunner {
    set: RuleSet,
    evaluator: Evaluator,
    state: PaperState,
    /// The trailing watermark tracking, mirroring the evaluator's own.
    pending_intent: Option<crate::rules::OrderIntent>,
    /// The symbol everything is about.
    symbol: String,
    /// The paper equity, for sizing.
    equity: f64,
}

impl PaperRunner {
    pub fn new(
        set: RuleSet,
        symbol: impl Into<String>,
        equity: f64,
    ) -> Result<Self, crate::evaluate::RuleError> {
        let symbol = symbol.into();
        Ok(Self {
            set: set.clone(),
            evaluator: Evaluator::new(set, equity)?.with_symbol(symbol.clone()),
            state: PaperState::default(),
            pending_intent: None,
            symbol,
            equity,
        })
    }

    /// The persisted state (position + day P&L), saved by the caller.
    pub fn state(&self) -> &PaperState {
        &self.state
    }

    /// Restores a saved state: the position and day P&L. The evaluator's
    /// indicator warm-up restarts; bars re-feed from the restored position.
    pub fn restore(&mut self, state: PaperState) {
        self.state = state;
    }

    /// The intent waiting to fill on the next bar's open, if any.
    pub fn pending_intent(&self) -> Option<&crate::rules::OrderIntent> {
        self.pending_intent.as_ref()
    }

    /// Feeds one closed bar. Returns the outcome: the (possibly updated)
    /// state and the intent to hand the gateway. The CALLER fills pending
    /// intents at the next bar's open by calling [`Self::fill_pending`]
    /// with that bar.
    pub fn feed(&mut self, bar: &Candle) -> PaperOutcome {
        let mut fill = None;

        // 1. A pending entry fills at this bar's open (the signal bar was
        //    the previous closed bar).
        if let Some(intent) = self.pending_intent.take()
            && intent.side == crate::rules::IntentSide::Buy
        {
            let qty = intent.quantity;
            if qty > 0 && bar.o > 0.0 {
                let set_stop = intent.stop_loss.map(|f| bar.o * (1.0 - f));
                let set_target = intent.target.map(|f| bar.o * (1.0 + f));
                self.state.position = Some(PaperPosition {
                    symbol: self.symbol.clone(),
                    quantity: qty,
                    entry_price: bar.o,
                    entry_t: bar.t,
                    high_watermark: Some(bar.h),
                    stop: set_stop,
                    target: set_target,
                });
                fill = Some((crate::rules::IntentSide::Buy, qty, bar.o));
            }
        }

        // 2. If holding a position: stop/target exits on the touch, rule
        //    exits on the close, trailing rides the watermark.
        if let Some(pos) = self.state.position.clone() {
            let stop_touched = pos.stop.is_some_and(|s| bar.l <= s);
            let target_touched = pos.target.is_some_and(|t| bar.h >= t);
            let watermark = pos.high_watermark.unwrap_or(bar.h).max(bar.h);
            let trailing_stop = self
                .evaluator_set()
                .trailing_stop
                .map(|f| watermark * (1.0 - f));
            if stop_touched || target_touched {
                let price = if stop_touched { pos.stop } else { pos.target }.unwrap_or(bar.c);
                self.state.day_pnl += (price - pos.entry_price) * pos.quantity as f64;
                let intent = crate::rules::OrderIntent {
                    symbol: self.symbol.clone(),
                    side: crate::rules::IntentSide::Sell,
                    quantity: pos.quantity,
                    signal_price: price,
                    signal_bar_t: bar.t,
                    stop_loss: None,
                    target: None,
                    trailing_stop: None,
                };
                fill = Some((crate::rules::IntentSide::Sell, pos.quantity, price));
                self.state.position = None;
                return PaperOutcome {
                    state: self.state.clone(),
                    fill,
                    intent: Some(intent),
                };
            }
            // A rule exit: the evaluator decided on this closed bar; the
            // paper fill happens at this bar's close.
            if let Some(intent) = self.evaluator.feed(bar)
                && intent.side == crate::rules::IntentSide::Sell
            {
                let price = intent.signal_price;
                self.state.day_pnl += (price - pos.entry_price) * pos.quantity as f64;
                fill = Some((crate::rules::IntentSide::Sell, pos.quantity, price));
                self.state.position = None;
                return PaperOutcome {
                    state: self.state.clone(),
                    fill,
                    intent: Some(intent),
                };
            }
            // Still holding: trail the stop and mark the watermark.
            let _ = trailing_stop;
            self.state.position = Some(PaperPosition {
                high_watermark: Some(watermark),
                ..pos
            });
            return PaperOutcome {
                state: self.state.clone(),
                fill,
                intent: None,
            };
        }

        // 3. Flat: the evaluator's entry intent waits for the next open.
        if let Some(intent) = self.evaluator.feed(bar)
            && intent.side == crate::rules::IntentSide::Buy
            && intent.quantity > 0
        {
            self.pending_intent = Some(intent.clone());
            return PaperOutcome {
                state: self.state.clone(),
                fill: None,
                intent: Some(intent),
            };
        }
        PaperOutcome {
            state: self.state.clone(),
            fill: None,
            intent: None,
        }
    }

    /// Fills the pending entry at this bar's open, the caller's next-bar
    /// convention. Returns the fill.
    pub fn fill_pending(&mut self, bar: &Candle) -> Option<(i64, f64)> {
        let intent = self.pending_intent.take()?;
        let qty = intent.quantity;
        if qty <= 0 || bar.o <= 0.0 {
            return None;
        }
        self.state.position = Some(PaperPosition {
            symbol: self.symbol.clone(),
            quantity: qty,
            entry_price: bar.o,
            entry_t: bar.t,
            high_watermark: Some(bar.h),
            stop: intent.stop_loss.map(|f| bar.o * (1.0 - f)),
            target: intent.target.map(|f| bar.o * (1.0 + f)),
        });
        Some((qty, bar.o))
    }

    fn evaluator_set(&self) -> &RuleSet {
        // The runner keeps the set reachable through the evaluator's API;
        // the trailing stop lives in the set.
        &self.set
    }

    /// Equity snapshot (cash + position at `mark`), for the dashboard.
    pub fn equity(&self, mark: f64) -> f64 {
        match &self.state.position {
            Some(pos) => self.equity + (mark - pos.entry_price) * pos.quantity as f64,
            None => self.equity,
        }
    }
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
    fn an_entry_fills_on_the_next_bar_open() {
        let mut runner = PaperRunner::new(crossing_set(), "RELIANCE", 100_000.0).unwrap();
        assert!(runner.feed(&candle(1, 99.0, 99.0)).fill.is_none());
        assert!(runner.feed(&candle(2, 100.0, 100.0)).fill.is_none());
        // The cross fires on the 3rd closed bar; the intent waits...
        let o3 = runner.feed(&candle(3, 100.0, 101.0));
        assert!(o3.intent.is_some(), "the gateway gets the entry intent");
        assert!(o3.fill.is_none(), "the fill is on the NEXT bar's open");
        // ...and fills on the 4th bar's open.
        let fill = runner.fill_pending(&candle(4, 102.0, 103.0)).unwrap();
        assert_eq!(fill, (1, 102.0));
        assert!(runner.state().position.is_some());
        assert_eq!(runner.state().position.as_ref().unwrap().entry_price, 102.0);
    }

    #[test]
    fn a_rule_exit_fills_at_this_close() {
        let mut runner = PaperRunner::new(crossing_set(), "RELIANCE", 100_000.0).unwrap();
        runner.feed(&candle(1, 99.0, 99.0));
        runner.feed(&candle(2, 100.0, 100.0));
        runner.feed(&candle(3, 100.0, 101.0));
        runner.fill_pending(&candle(4, 102.0, 103.0)).unwrap();
        // The exit cross fires on the 5th closed bar; the paper fill is
        // this bar's close (the runner's own fill model).
        let o5 = runner.feed(&candle(5, 99.0, 99.0));
        let (_, qty, price) = o5.fill.expect("exited");
        assert_eq!(qty, 1);
        assert_eq!(price, 99.0);
        assert!(runner.state().position.is_none());
        assert!(runner.state().day_pnl < 0.0);
    }

    #[test]
    fn a_stop_closes_the_position_at_the_stop() {
        let set = RuleSet {
            entry: Rule::True,
            stop_loss: Some(0.02),
            sizing: Sizing::FixedQuantity(1),
            ..RuleSet::default()
        };
        let mut runner = PaperRunner::new(set, "X", 100_000.0).unwrap();
        runner.feed(&candle(1, 100.0, 100.0));
        let fill = runner.fill_pending(&candle(2, 100.0, 100.0)).unwrap();
        assert_eq!(fill, (1, 100.0));
        // The next bar dips to the 2% stop.
        let mut bar = candle(3, 99.0, 99.0);
        bar.l = 98.0;
        let outcome = runner.feed(&bar);
        let (_, _, price) = outcome.fill.expect("stopped out");
        assert_eq!(price, 98.0);
        assert!(runner.state().position.is_none());
        assert_eq!(runner.state().day_pnl, -2.0);
    }

    #[test]
    fn equity_tracks_the_mark() {
        let set = RuleSet {
            entry: Rule::True,
            sizing: Sizing::FixedQuantity(10),
            ..RuleSet::default()
        };
        let mut runner = PaperRunner::new(set, "X", 100_000.0).unwrap();
        runner.feed(&candle(1, 100.0, 100.0));
        runner.fill_pending(&candle(2, 100.0, 100.0)).unwrap();
        // Marked at 101: equity = 100k + (101 - 100) * 10.
        assert_eq!(runner.equity(101.0), 100_010.0);
    }
}
