//! The paper desk: every running paper strategy behind one gateway.
//!
//! The harness (the app) owns the bar streams and the persistence; the desk
//! owns the decisions. For each closed bar of a strategy's stream it
//! settles that strategy's queued orders at the bar's open, lets the runner
//! decide on the bar, and routes the resulting intent through
//! [`OrderGateway::submit`]. Nothing here reaches a real broker: the only
//! route is the paper fill model.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use nivesh_core::{Candle, Side};
use nivesh_risk::{RiskEngine, RiskLimits, paise};
use nivesh_strategy::{IntentSide, PaperRunner, PaperState};

use crate::{
    AccountView, AuditSink, Fill, GatewayError, NextOpen, OrderGateway, OrderIntent, PaperBroker,
};

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
    /// Strategies' own limits (they can only tighten the global ones).
    limits: HashMap<i64, RiskLimits>,
    /// The latest close seen per symbol: the mark for MTM and exposure.
    marks: HashMap<String, f64>,
}

impl PaperDesk {
    /// A desk filling at the next bar's open with the default cost model
    /// (the backtester's default too), auditing to `audit`.
    pub fn new(engine: RiskEngine, audit: Arc<dyn AuditSink>) -> Self {
        Self::with_broker(engine, Arc::new(NextOpen::default()), audit)
    }

    pub fn with_broker(
        engine: RiskEngine,
        paper: Arc<dyn PaperBroker>,
        audit: Arc<dyn AuditSink>,
    ) -> Self {
        Self {
            gateway: OrderGateway::new(engine, paper, audit),
            runners: BTreeMap::new(),
            limits: HashMap::new(),
            marks: HashMap::new(),
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

    /// Sets (or clears) a strategy's own limits.
    pub fn set_strategy_limits(&mut self, strategy_id: i64, limits: Option<RiskLimits>) {
        match limits {
            Some(l) => self.limits.insert(strategy_id, l),
            None => self.limits.remove(&strategy_id),
        };
    }

    /// The account as the risk check sees it for `strategy_id` on `symbol`:
    /// positions marked at the latest closes seen.
    pub fn account_view(&self, strategy_id: i64, symbol: &str) -> AccountView {
        let mut view = AccountView {
            strategy_limits: self.limits.get(&strategy_id).cloned(),
            ..AccountView::default()
        };
        let mut day_pnl = 0.0;
        for (id, runner) in &self.runners {
            let state = runner.state();
            day_pnl += state.day_pnl;
            let Some(pos) = &state.position else { continue };
            view.open_positions += 1;
            let mark = self
                .marks
                .get(&pos.symbol)
                .copied()
                .unwrap_or(pos.entry_price);
            day_pnl += (mark - pos.entry_price) * pos.quantity as f64;
            if pos.symbol == symbol {
                view.symbol_exposure_paise += paise(mark) * pos.quantity;
                if *id == strategy_id {
                    view.position_quantity = pos.quantity;
                }
            }
        }
        view.day_pnl_paise = paise(day_pnl);
        view
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
        for settled in self
            .gateway
            .on_closed_bar(strategy_id, &symbol, bar, now_ms)
        {
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

        // 2. The bar's close marks the symbol for MTM and exposure.
        self.marks.insert(symbol.clone(), bar.c);

        // 3. The runner decides on the closed bar; the gateway checks it
        //    against the whole book.
        let intent = self.runners.get_mut(&strategy_id)?.on_bar(&bar);
        let mut placed = None;
        if let Some(intent) = intent {
            let account = self.account_view(strategy_id, &symbol);
            let order = OrderIntent {
                strategy_id,
                symbol: symbol.clone(),
                side: side(intent.side),
                quantity: intent.quantity,
                signal_price: intent.signal_price,
                signal_bar_t: intent.signal_bar_t,
            };
            match self.gateway.submit(&order, &account, now_ms) {
                Ok(decision) => placed = Some(decision),
                Err(err) => {
                    if let Some(runner) = self.runners.get_mut(&strategy_id) {
                        runner.on_rejected();
                    }
                    refused.push(err);
                }
            }
        }
        let runner = self.runners.get(&strategy_id)?;

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
    use nivesh_risk::RiskCheck;
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
            RiskEngine::new(RiskLimits {
                trading_window: None,
                ..RiskLimits::default()
            }),
            Arc::new(NextOpen::FREE),
            Arc::new(crate::MemoryAudit::default()),
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
        assert!(
            matches!(&r0.refused[..], [GatewayError::RiskRejected(r)] if r.check == RiskCheck::Quantity)
        );
        assert!(r0.placed.is_none());
        let b1 = bar(1, 100.0, 100.0);
        let r1 = desk.on_closed_bar(7, b1, b1.t).unwrap();
        assert!(r1.fills.is_empty());
        assert!(r1.state.position.is_none());
        assert_eq!(
            r1.refused,
            vec![r0.refused[0].clone()],
            "tries again, refused again"
        );
    }

    #[test]
    fn a_strategy_can_still_exit_after_the_daily_loss_limit_is_hit() {
        // Long 100 at 100, then the price halves: a ₹5,000 MTM loss, far
        // over a ₹1,000 daily-loss limit. The exit must still go through.
        let set = RuleSet {
            exit: Rule::Cmp {
                left: nivesh_strategy::Operand::Close,
                cmp: nivesh_strategy::Comparison::Less,
                right: nivesh_strategy::Operand::Constant(60.0),
            },
            ..always_in(100)
        };
        let mut desk = PaperDesk::with_broker(
            RiskEngine::new(RiskLimits {
                trading_window: None,
                max_daily_loss_paise: 1_000 * 100,
                ..RiskLimits::default()
            }),
            Arc::new(NextOpen::FREE),
            Arc::new(crate::MemoryAudit::default()),
        );
        desk.start(1, PaperRunner::new(set, "NSE:RELIANCE", 100_000.0).unwrap());
        desk.on_closed_bar(1, bar(0, 100.0, 100.0), T0);
        desk.on_closed_bar(1, bar(1, 100.0, 100.0), T0 + DAY); // filled at 100
        let view = desk.account_view(1, "NSE:RELIANCE");
        assert_eq!((view.position_quantity, view.open_positions), (100, 1));
        let r = desk
            .on_closed_bar(1, bar(2, 60.0, 50.0), T0 + 2 * DAY)
            .unwrap();
        let view = desk.account_view(1, "NSE:RELIANCE");
        assert_eq!(view.day_pnl_paise, -5_000 * 100, "MTM at the 50 close");
        assert_eq!(view.symbol_exposure_paise, 100 * 50 * 100);
        assert!(
            r.refused.is_empty(),
            "the exit is approved: {:?}",
            r.refused
        );
        assert!(r.placed.is_some());
        let r = desk
            .on_closed_bar(1, bar(3, 51.0, 52.0), T0 + 3 * DAY)
            .unwrap();
        assert_eq!(r.fills.len(), 1, "sold at the next open");
        // Re-entry is new risk: refused while the loss stands.
        assert!(
            matches!(&r.refused[..], [GatewayError::RiskRejected(x)] if x.check == RiskCheck::DailyLoss),
            "{:?}",
            r.refused
        );
    }

    #[test]
    fn strategy_limits_tighten_the_desk() {
        let mut desk = desk_with(7, always_in(10));
        desk.set_strategy_limits(
            7,
            Some(RiskLimits {
                max_order_quantity: 5,
                ..RiskLimits::default()
            }),
        );
        let r = desk.on_closed_bar(7, bar(0, 100.0, 100.0), T0).unwrap();
        assert!(
            matches!(&r.refused[..], [GatewayError::RiskRejected(x)] if x.check == RiskCheck::Quantity)
        );
        desk.set_strategy_limits(7, None);
        let r = desk
            .on_closed_bar(7, bar(1, 100.0, 100.0), T0 + DAY)
            .unwrap();
        assert!(r.placed.is_some());
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
