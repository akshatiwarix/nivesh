//! The paper desk: every running paper strategy behind one gateway.
//!
//! The harness (the app) owns the bar streams and the persistence; the desk
//! owns the decisions. For each closed bar of a strategy's stream it
//! settles that strategy's queued orders at the bar's open, lets the runner
//! decide on the bar, and routes the resulting intent through
//! [`OrderGateway::submit`]. Nothing here reaches a real broker: the only
//! route is the paper fill model.

use std::collections::BTreeMap;
use std::sync::Arc;

use nivesh_core::{Candle, Side};
use nivesh_risk::RiskEngine;
use nivesh_strategy::{IntentSide, PaperRunner, PaperState};

use crate::{Fill, GatewayError, NextOpen, OrderGateway, OrderIntent, PaperBroker};

/// What one closed bar did to one strategy.
#[derive(Debug, Clone, PartialEq)]
pub struct StepReport {
    /// Orders from earlier bars that filled at this bar's open.
    pub fills: Vec<Fill>,
    /// The risk decision id, when this bar's intent was approved and queued.
    pub placed: Option<u64>,
    /// Why the gateway refused this bar's intent, or dropped a queued order.
    pub refused: Vec<GatewayError>,
    /// The runner's state after the bar: persist it.
    pub state: PaperState,
}

pub struct PaperDesk {
    gateway: OrderGateway,
    runners: BTreeMap<i64, PaperRunner>,
}

impl PaperDesk {
    /// A desk filling at the next bar's open with the default cost model
    /// (the backtester's default too).
    pub fn new(engine: RiskEngine) -> Self {
        Self::with_broker(engine, Arc::new(NextOpen::default()))
    }

    pub fn with_broker(engine: RiskEngine, paper: Arc<dyn PaperBroker>) -> Self {
        Self {
            gateway: OrderGateway::new(engine, paper),
            runners: BTreeMap::new(),
        }
    }

    /// Starts (or restarts) a strategy. Anything it had queued is dropped.
    pub fn start(&mut self, strategy_id: i64, runner: PaperRunner) {
        self.gateway.forget(strategy_id);
        self.runners.insert(strategy_id, runner);
    }

    /// Stops a strategy, dropping its queued orders. Returns its last state.
    pub fn stop(&mut self, strategy_id: i64) -> Option<PaperState> {
        self.gateway.forget(strategy_id);
        self.runners.remove(&strategy_id).map(|r| r.state().clone())
    }

    pub fn is_running(&self, strategy_id: i64) -> bool {
        self.runners.contains_key(&strategy_id)
    }

    pub fn state(&self, strategy_id: i64) -> Option<&PaperState> {
        self.runners.get(&strategy_id).map(PaperRunner::state)
    }

    /// Orders approved and waiting for their stream's next bar.
    pub fn queued(&self) -> usize {
        self.gateway.queued()
    }

    /// One closed bar of `strategy_id`'s stream. `None` if it isn't running.
    pub fn on_closed_bar(
        &mut self,
        strategy_id: i64,
        bar: Candle,
        now_ms: i64,
    ) -> Option<StepReport> {
        let runner = self.runners.get_mut(&strategy_id)?;
        let symbol = runner.symbol().to_owned();
        let mut fills = Vec::new();
        let mut refused = Vec::new();

        // 1. Orders from the previous bar settle at this bar's open.
        for settled in self.gateway.on_closed_bar(strategy_id, &symbol, bar) {
            match settled {
                Ok(fill) => {
                    runner.on_fill(
                        intent_side(fill.side),
                        fill.quantity,
                        fill.price,
                        fill.charges,
                        fill.bar_t,
                    );
                    fills.push(fill);
                }
                Err(err) => {
                    runner.on_rejected();
                    refused.push(err);
                }
            }
        }

        // 2. The risk context sees every strategy's book.
        let open_positions = self
            .runners
            .values()
            .filter(|r| r.state().position.is_some())
            .count();
        let day_pnl = self.runners.values().map(|r| r.state().day_pnl).sum();
        self.gateway.set_account_state(open_positions, day_pnl);

        // 3. The runner decides on the closed bar; the gateway checks it.
        let runner = self.runners.get_mut(&strategy_id)?;
        let mut placed = None;
        if let Some(intent) = runner.on_bar(&bar) {
            let order = OrderIntent {
                strategy_id,
                symbol: symbol.clone(),
                side: side(intent.side),
                quantity: intent.quantity,
                signal_price: intent.signal_price,
                signal_bar_t: intent.signal_bar_t,
            };
            match self.gateway.submit(&order, now_ms) {
                Ok(decision) => placed = Some(decision),
                Err(err) => {
                    runner.on_rejected();
                    refused.push(err);
                }
            }
        }

        Some(StepReport {
            fills,
            placed,
            refused,
            state: runner.state().clone(),
        })
    }
}

fn side(side: IntentSide) -> Side {
    match side {
        IntentSide::Buy => Side::Buy,
        IntentSide::Sell => Side::Sell,
    }
}

fn intent_side(side: Side) -> IntentSide {
    match side {
        Side::Buy => IntentSide::Buy,
        Side::Sell => IntentSide::Sell,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nivesh_risk::RiskLimits;
    use nivesh_strategy::{Rule, RuleSet, Sizing};

    const DAY: i64 = 86_400_000;
    const T0: i64 = 1_795_150_800_000;

    fn bar(i: i64, o: f64, c: f64) -> Candle {
        Candle {
            t: T0 + i * DAY,
            o,
            h: o.max(c),
            l: o.min(c),
            c,
            v: 100.0,
        }
    }

    fn always_in(qty: i64) -> RuleSet {
        RuleSet {
            entry: Rule::True,
            exit: Rule::False,
            sizing: Sizing::FixedQuantity(qty),
            ..RuleSet::default()
        }
    }

    /// A desk without costs, so prices in the tests are the bars' own.
    fn desk_with(id: i64, set: RuleSet) -> PaperDesk {
        let mut desk = PaperDesk::with_broker(
            RiskEngine::new(RiskLimits::default()),
            Arc::new(NextOpen::FREE),
        );
        desk.start(
            id,
            PaperRunner::new(set, "NSE:RELIANCE", 100_000.0).unwrap(),
        );
        desk
    }

    #[test]
    fn a_signal_is_placed_then_filled_on_the_next_bar() {
        let mut desk = desk_with(7, always_in(1));
        let b0 = bar(0, 100.0, 100.0);
        let r0 = desk.on_closed_bar(7, b0, b0.t).unwrap();
        assert!(r0.placed.is_some());
        assert!(r0.fills.is_empty());
        assert!(r0.state.position.is_none());

        let b1 = bar(1, 101.0, 102.0);
        let r1 = desk.on_closed_bar(7, b1, b1.t).unwrap();
        assert_eq!(r1.fills.len(), 1);
        assert_eq!(r1.fills[0].price, 101.0);
        let pos = r1.state.position.unwrap();
        assert_eq!((pos.quantity, pos.entry_price), (1, 101.0));
        assert!(r1.placed.is_none(), "no pyramiding");
    }

    #[test]
    fn a_risk_refusal_is_reported_and_the_runner_stays_flat() {
        // 10,000 shares is over the 1,000-share cap.
        let mut desk = desk_with(7, always_in(10_000));
        let b0 = bar(0, 100.0, 100.0);
        let r0 = desk.on_closed_bar(7, b0, b0.t).unwrap();
        assert_eq!(r0.refused, vec![GatewayError::RiskRejected]);
        assert!(r0.placed.is_none());
        let b1 = bar(1, 100.0, 100.0);
        let r1 = desk.on_closed_bar(7, b1, b1.t).unwrap();
        assert!(r1.fills.is_empty());
        assert!(r1.state.position.is_none());
        assert_eq!(
            r1.refused,
            vec![GatewayError::RiskRejected],
            "tries again, refused again"
        );
    }

    #[test]
    fn stopping_drops_the_queued_order() {
        let mut desk = desk_with(7, always_in(1));
        let b0 = bar(0, 100.0, 100.0);
        desk.on_closed_bar(7, b0, b0.t).unwrap();
        assert_eq!(desk.queued(), 1);
        assert!(desk.stop(7).is_some());
        assert_eq!(desk.queued(), 0);
        assert!(!desk.is_running(7));
        assert!(
            desk.on_closed_bar(7, bar(1, 100.0, 100.0), T0 + DAY)
                .is_none()
        );
    }

    #[test]
    fn a_stopped_out_strategy_holds_nothing_and_has_the_loss() {
        let set = RuleSet {
            stop_loss: Some(0.05),
            ..always_in(10)
        };
        let mut desk = desk_with(1, set);
        let bars = [
            bar(0, 100.0, 100.0),
            bar(1, 100.0, 100.0), // entry fills at 100
            bar(2, 100.0, 90.0),  // the 5% stop (on the signal close) is touched
            bar(3, 91.0, 92.0),   // the exit fills at this open
        ];
        let mut last = None;
        for b in bars {
            last = desk.on_closed_bar(1, b, b.t);
        }
        let state = last.unwrap().state;
        assert!(state.position.is_none());
        assert_eq!(state.realised_pnl, -90.0);
    }
}
