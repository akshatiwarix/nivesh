//! The paper trading runner: one strategy on one symbol's closed bars.
//! Everything it produces is **PAPER**: simulated fills on simulated
//! money, labelled as such everywhere it shows up.
//!
//! The runner never fills its own orders. It turns each closed bar into at
//! most one [`OrderIntent`]; the harness hands that to the order gateway
//! (`nivesh-exec`), which risk-checks it, queues it and fills it at the next
//! bar's open, then reports the fill back through [`PaperRunner::on_fill`].
//! The decisions and the fill convention are the backtester's, so a paper
//! run over the same bars trades exactly like a backtest (tested in
//! `nivesh-exec`).
//!
//! [`PaperState`] is everything that must survive a restart. The indicator
//! state does not need to: the harness re-warms it from history
//! ([`PaperRunner::warm_up`]) and the evaluator then adopts the saved
//! position.

use nivesh_core::Candle;
use serde::{Deserialize, Serialize};

use crate::evaluate::{Evaluator, RuleError};
use crate::rules::{IntentSide, OrderIntent, RuleSet};

/// How many fills the state keeps for the dashboard (oldest dropped).
pub const MAX_FILLS: usize = 200;

/// IST is UTC+05:30; paper "days" follow the Indian trading day.
const IST_OFFSET_MS: i64 = 19_800_000;
const DAY_MS: i64 = 86_400_000;

/// The paper position a running strategy holds (long-only in v1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaperPosition {
    pub symbol: String,
    pub quantity: i64,
    /// The paper fill price, rupees.
    pub entry_price: f64,
    /// UTC ms of the bar whose open filled it.
    pub entry_t: i64,
}

/// One paper fill, for the dashboard's order list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaperFill {
    /// UTC ms of the bar whose open filled it.
    pub t: i64,
    pub side: IntentSide,
    pub quantity: i64,
    /// Rupees.
    pub price: f64,
    /// Realised P&L of the round trip, on the closing fill.
    pub pnl: Option<f64>,
}

/// Everything a paper strategy needs to resume after a restart.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PaperState {
    pub position: Option<PaperPosition>,
    /// Realised P&L since the strategy first started, rupees.
    pub realised_pnl: f64,
    /// Realised P&L of the current IST day, rupees.
    pub day_pnl: f64,
    /// The IST day `day_pnl` belongs to (days since the epoch).
    pub day: i64,
    /// The newest closed bar the runner has decided on. Older bars are
    /// ignored, so a bar replayed after a reconnect is never traded twice.
    pub last_bar_t: Option<i64>,
    /// The newest fills, oldest first, at most [`MAX_FILLS`].
    pub fills: Vec<PaperFill>,
}

/// The runner: one evaluator, one position, one symbol.
pub struct PaperRunner {
    evaluator: Evaluator,
    state: PaperState,
    symbol: String,
    /// Starting paper equity, rupees (sizing and the equity mark).
    equity: f64,
}

impl PaperRunner {
    pub fn new(set: RuleSet, symbol: impl Into<String>, equity: f64) -> Result<Self, RuleError> {
        let symbol = symbol.into();
        Ok(Self {
            evaluator: Evaluator::new(set, equity)?.with_symbol(symbol.clone()),
            state: PaperState::default(),
            symbol,
            equity,
        })
    }

    /// Restores a saved state. Call [`Self::warm_up`] afterwards (or with no
    /// bars) so the evaluator adopts the saved position.
    pub fn with_state(mut self, state: PaperState) -> Self {
        self.state = state;
        self.sync_evaluator();
        self
    }

    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    /// The state to persist after every bar.
    pub fn state(&self) -> &PaperState {
        &self.state
    }

    /// Feeds already-closed history bars to the indicators without trading
    /// them, then aligns the evaluator with the saved position. Bars that
    /// closed while the app was not running are skipped, never traded late.
    pub fn warm_up(&mut self, bars: &[Candle]) {
        for bar in bars {
            let _ = self.evaluator.feed(bar);
        }
        if let Some(last) = bars.last() {
            self.state.last_bar_t = Some(self.state.last_bar_t.map_or(last.t, |t| t.max(last.t)));
        }
        self.sync_evaluator();
    }

    /// Decides on one closed bar. Returns the intent for the gateway, if
    /// any. A bar at or before the last one decided on returns `None`.
    pub fn on_bar(&mut self, bar: &Candle) -> Option<OrderIntent> {
        if self.state.last_bar_t.is_some_and(|t| bar.t <= t) {
            return None;
        }
        self.state.last_bar_t = Some(bar.t);
        self.roll_day(bar.t);
        let intent = self.evaluator.feed(bar)?;
        // The backtester's relevance rule: no pyramiding, no exits while
        // flat.
        let relevant = match intent.side {
            IntentSide::Buy => self.state.position.is_none() && intent.quantity > 0,
            IntentSide::Sell => self.state.position.is_some(),
        };
        if !relevant {
            self.sync_evaluator();
            return None;
        }
        Some(intent)
    }

    /// The gateway filled an intent this runner emitted.
    pub fn on_fill(&mut self, side: IntentSide, quantity: i64, price: f64, t: i64) {
        self.roll_day(t);
        let pnl = match side {
            IntentSide::Buy => {
                self.state.position = Some(PaperPosition {
                    symbol: self.symbol.clone(),
                    quantity,
                    entry_price: price,
                    entry_t: t,
                });
                None
            }
            IntentSide::Sell => self.state.position.take().map(|pos| {
                let pnl = (price - pos.entry_price) * pos.quantity as f64;
                self.state.realised_pnl += pnl;
                self.state.day_pnl += pnl;
                pnl
            }),
        };
        self.state.fills.push(PaperFill {
            t,
            side,
            quantity,
            price,
            pnl,
        });
        if self.state.fills.len() > MAX_FILLS {
            let excess = self.state.fills.len() - MAX_FILLS;
            self.state.fills.drain(..excess);
        }
    }

    /// The gateway refused (or dropped) an intent: the evaluator assumed
    /// it would fill, so bring it back in line with the real position.
    pub fn on_rejected(&mut self) {
        self.sync_evaluator();
    }

    /// Paper equity marked at `mark` (rupees).
    pub fn equity(&self, mark: f64) -> f64 {
        let open = self
            .state
            .position
            .as_ref()
            .map_or(0.0, |p| (mark - p.entry_price) * p.quantity as f64);
        self.equity + self.state.realised_pnl + open
    }

    fn sync_evaluator(&mut self) {
        self.evaluator
            .sync_position(self.state.position.as_ref().map(|p| p.entry_price));
    }

    fn roll_day(&mut self, t: i64) {
        let day = (t + IST_OFFSET_MS).div_euclid(DAY_MS);
        if day != self.state.day {
            self.state.day = day;
            self.state.day_pnl = 0.0;
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
    fn a_cross_emits_an_entry_intent_and_the_fill_opens_the_position() {
        let mut runner = PaperRunner::new(crossing_set(), "RELIANCE", 100_000.0).unwrap();
        assert!(runner.on_bar(&candle(1, 99.0, 99.0)).is_none());
        assert!(runner.on_bar(&candle(2, 100.0, 100.0)).is_none());
        let intent = runner.on_bar(&candle(3, 100.0, 101.0)).expect("entry");
        assert_eq!(intent.side, IntentSide::Buy);
        assert_eq!(intent.signal_bar_t, 3);
        assert!(runner.state().position.is_none(), "no fill until reported");
        runner.on_fill(IntentSide::Buy, 1, 102.0, 4);
        let pos = runner.state().position.as_ref().unwrap();
        assert_eq!((pos.quantity, pos.entry_price, pos.entry_t), (1, 102.0, 4));
    }

    #[test]
    fn an_exit_fill_realises_the_pnl() {
        let mut runner = PaperRunner::new(crossing_set(), "RELIANCE", 100_000.0).unwrap();
        runner.on_bar(&candle(1, 99.0, 99.0));
        runner.on_bar(&candle(2, 100.0, 100.0));
        runner.on_bar(&candle(3, 100.0, 101.0)).unwrap();
        runner.on_fill(IntentSide::Buy, 1, 102.0, 4);
        assert!(runner.on_bar(&candle(4, 102.0, 103.0)).is_none());
        let exit = runner.on_bar(&candle(5, 99.0, 99.0)).expect("exit");
        assert_eq!(exit.side, IntentSide::Sell);
        runner.on_fill(IntentSide::Sell, 1, 98.0, 6);
        assert!(runner.state().position.is_none());
        assert_eq!(runner.state().realised_pnl, -4.0);
        assert_eq!(runner.state().fills.len(), 2);
        assert_eq!(runner.state().fills[1].pnl, Some(-4.0));
        assert_eq!(runner.equity(0.0), 99_996.0);
    }

    #[test]
    fn a_replayed_bar_is_never_decided_twice() {
        let set = RuleSet {
            entry: Rule::True,
            sizing: Sizing::FixedQuantity(1),
            ..RuleSet::default()
        };
        let mut runner = PaperRunner::new(set, "X", 100_000.0).unwrap();
        assert!(runner.on_bar(&candle(10, 100.0, 100.0)).is_some());
        runner.on_rejected();
        assert!(runner.on_bar(&candle(10, 100.0, 100.0)).is_none());
        assert!(runner.on_bar(&candle(9, 100.0, 100.0)).is_none());
    }

    #[test]
    fn a_rejected_entry_leaves_the_runner_flat_and_ready_to_retry() {
        let set = RuleSet {
            entry: Rule::True,
            sizing: Sizing::FixedQuantity(1),
            ..RuleSet::default()
        };
        let mut runner = PaperRunner::new(set, "X", 100_000.0).unwrap();
        assert!(runner.on_bar(&candle(1, 100.0, 100.0)).is_some());
        runner.on_rejected();
        // The evaluator would think it was long without the resync, and
        // never enter again.
        let again = runner.on_bar(&candle(2, 100.0, 100.0)).expect("re-entry");
        assert_eq!(again.side, IntentSide::Buy);
    }

    #[test]
    fn state_round_trips_and_resumes_the_position() {
        let mut runner = PaperRunner::new(crossing_set(), "RELIANCE", 100_000.0).unwrap();
        runner.on_bar(&candle(1, 99.0, 99.0));
        runner.on_bar(&candle(2, 100.0, 100.0));
        runner.on_bar(&candle(3, 100.0, 101.0)).unwrap();
        runner.on_fill(IntentSide::Buy, 1, 102.0, 4);
        runner.on_bar(&candle(4, 102.0, 103.0));
        let json = serde_json::to_string(runner.state()).unwrap();

        // A restart: a fresh runner, the saved state, re-warmed history.
        let saved: PaperState = serde_json::from_str(&json).unwrap();
        let mut resumed = PaperRunner::new(crossing_set(), "RELIANCE", 100_000.0)
            .unwrap()
            .with_state(saved.clone());
        resumed.warm_up(&[
            candle(1, 99.0, 99.0),
            candle(2, 100.0, 100.0),
            candle(3, 100.0, 101.0),
            candle(4, 102.0, 103.0),
        ]);
        assert_eq!(resumed.state(), &saved, "warm-up changes no state");
        // The evaluator knows it is long, so the exit cross fires.
        let exit = resumed.on_bar(&candle(5, 99.0, 99.0)).expect("exit");
        assert_eq!(exit.side, IntentSide::Sell);
    }

    #[test]
    fn warm_up_skips_bars_that_closed_while_the_app_was_off() {
        let set = RuleSet {
            entry: Rule::True,
            sizing: Sizing::FixedQuantity(1),
            ..RuleSet::default()
        };
        let mut runner = PaperRunner::new(set, "X", 100_000.0).unwrap();
        runner.warm_up(&[candle(1, 100.0, 100.0), candle(2, 100.0, 100.0)]);
        assert_eq!(runner.state().last_bar_t, Some(2));
        assert!(runner.on_bar(&candle(2, 100.0, 100.0)).is_none());
        assert!(runner.on_bar(&candle(3, 100.0, 100.0)).is_some());
    }

    #[test]
    fn the_day_pnl_resets_at_the_ist_day_boundary() {
        let mut runner = PaperRunner::new(crossing_set(), "RELIANCE", 100_000.0).unwrap();
        // 2026-11-20 10:30 IST and 2026-11-21 10:30 IST.
        let day1 = 1_795_150_800_000;
        let day2 = day1 + DAY_MS;
        runner.on_fill(IntentSide::Buy, 1, 100.0, day1);
        runner.on_fill(IntentSide::Sell, 1, 110.0, day1 + 60_000);
        assert_eq!(runner.state().day_pnl, 10.0);
        runner.on_fill(IntentSide::Buy, 1, 100.0, day2);
        assert_eq!(runner.state().day_pnl, 0.0, "a new day");
        assert_eq!(runner.state().realised_pnl, 10.0, "the total carries");
    }

    #[test]
    fn an_old_saved_state_still_loads() {
        // Part 5's first cut stored only the day P&L.
        let state: PaperState = serde_json::from_str(r#"{"day_pnl":0.0}"#).unwrap();
        assert_eq!(state, PaperState::default());
    }
}
