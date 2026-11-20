//! The paper desk: every running paper strategy behind one gateway.
//!
//! The harness (the app) owns the bar streams and the persistence; the desk
//! owns the decisions. For each closed bar of a strategy's stream it
//! settles that strategy's queued orders at the bar's open, lets the runner
//! decide on the bar, and routes the resulting intent through
//! [`OrderGateway::submit`]. Paper strategies fill through the paper fill
//! model. An armed strategy's approved orders come back in the report as
//! [`LiveDispatch`]es for the harness to send; the desk itself never
//! awaits a broker.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use nivesh_broker::OrderAck;
use nivesh_core::{Candle, Freshness, Side};
use nivesh_risk::{RiskEngine, RiskLimits, paise};
use nivesh_strategy::{IntentSide, PaperRunner, PaperState};

use crate::arming::{ArmConditions, ArmError};
use crate::kill::{KillMonitor, KillReason, KillState, KillTriggers, Observation};
use crate::{
    AccountView, AuditSink, Fill, GatewayError, LiveDispatch, LiveRoute, NextOpen, OrderGateway,
    OrderIntent, PaperBroker,
};

/// What one closed bar did to one strategy.
#[derive(Debug)]
pub struct StepReport {
    /// Orders from earlier bars that filled at this bar's open.
    pub fills: Vec<Fill>,
    /// The risk decision id, when this bar's intent was approved and queued.
    pub placed: Option<u64>,
    /// Why the gateway refused this bar's intent, or dropped a queued order.
    pub refused: Vec<GatewayError>,
    /// The runner's state after the bar: persist it.
    pub state: PaperState,
    /// Set when this bar engaged the kill switch: persist it.
    pub kill: Option<KillState>,
    /// Live orders to send (armed strategies only): await each
    /// [`LiveDispatch::send`] outside the desk's lock, then report the
    /// outcome with [`PaperDesk::on_live_outcome`].
    pub dispatches: Vec<LiveDispatch>,
}

/// The account's usage against the limits, for the RISK view.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeskUsage {
    /// Today's P&L, realised plus MTM, paise.
    pub day_pnl_paise: i64,
    pub open_positions: usize,
    /// The largest notional held in one symbol, paise.
    pub largest_exposure_paise: i64,
    /// Orders approved since IST midnight.
    pub orders_today: usize,
    /// Orders approved in the last minute.
    pub orders_last_minute: usize,
}

/// IST is UTC+05:30.
const IST_OFFSET_MS: i64 = 19_800_000;
const DAY_MS: i64 = 86_400_000;

pub struct PaperDesk {
    gateway: OrderGateway,
    runners: BTreeMap<i64, PaperRunner>,
    /// Strategies' own limits (they can only tighten the global ones).
    limits: HashMap<i64, RiskLimits>,
    /// The latest close seen per symbol: the mark for MTM and exposure.
    marks: HashMap<String, f64>,
    /// The kill switch's automatic triggers.
    monitor: KillMonitor,
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
        // The kill switch trips at the same daily loss the risk check
        // refuses new risk at.
        let monitor = KillMonitor::new(KillTriggers {
            max_daily_loss_paise: engine.limits.max_daily_loss_paise,
            ..KillTriggers::default()
        });
        Self {
            gateway: OrderGateway::new(engine, paper, audit),
            runners: BTreeMap::new(),
            limits: HashMap::new(),
            marks: HashMap::new(),
            monitor,
        }
    }

    pub fn kill_state(&self) -> &KillState {
        self.gateway.kill_state()
    }

    /// Restores the persisted kill switch at startup.
    pub fn restore_kill(&mut self, state: KillState) {
        self.gateway.restore_kill(state);
    }

    /// Engages the kill switch (the one-keystroke action). Returns the
    /// cancelled orders' decision ids, or `None` if it was already engaged.
    pub fn engage_kill(
        &mut self,
        reason: KillReason,
        detail: &str,
        now_ms: i64,
    ) -> Option<Vec<u64>> {
        let cancelled = self.gateway.engage_kill(reason, detail, now_ms)?;
        // The cancelled orders will never fill: every runner goes back to
        // the position it really holds.
        for runner in self.runners.values_mut() {
            runner.on_rejected();
        }
        Some(cancelled)
    }

    /// Resets the kill switch with the typed phrase, after the cool-down.
    pub fn reset_kill(&mut self, typed: &str, now_ms: i64) -> Result<(), GatewayError> {
        self.gateway.reset_kill(typed, now_ms)
    }

    /// Reports something the harness saw (a disconnect, stale data, clock
    /// drift); engages the kill switch if it calls for it. Returns the new
    /// state when it engaged.
    pub fn observe(&mut self, seen: Observation, now_ms: i64) -> Option<KillState> {
        let (reason, detail) = self.monitor.observe(seen, now_ms)?;
        self.engage_kill(reason, &detail, now_ms)
            .map(|_| self.gateway.kill_state().clone())
    }

    /// Pauses every strategy a reconciliation found a mismatch for: their
    /// intents are refused until resumed. Returns the newly paused ids.
    pub fn apply_reconciliation(&mut self, found: &crate::Reconciliation, now_ms: i64) -> Vec<i64> {
        let mut paused = Vec::new();
        for strategy_id in found.strategies_to_pause() {
            let reason = found
                .mismatches
                .iter()
                .find(|m| m.strategies().contains(&strategy_id))
                .map(|m| serde_json::to_string(m).unwrap_or_default())
                .unwrap_or_else(|| "reconciliation mismatch".into());
            if self.gateway.pause(strategy_id, &reason, now_ms) {
                if let Some(runner) = self.runners.get_mut(&strategy_id) {
                    runner.on_rejected();
                }
                paused.push(strategy_id);
            }
        }
        paused
    }

    /// Resumes a paused strategy (the user's deliberate choice).
    pub fn resume(&mut self, strategy_id: i64, now_ms: i64) -> Result<(), GatewayError> {
        self.gateway.resume(strategy_id, now_ms)
    }

    pub fn paused(&self, strategy_id: i64) -> Option<&str> {
        self.gateway.paused(strategy_id)
    }

    /// Starts (or restarts) a strategy, in paper. Anything it had queued
    /// is dropped.
    pub fn start(&mut self, strategy_id: i64, runner: PaperRunner, now_ms: i64) {
        self.gateway.forget(strategy_id, now_ms);
        self.runners.insert(strategy_id, runner);
    }

    /// Stops a strategy, dropping its queued orders and disarming it.
    /// Returns its last state.
    pub fn stop(&mut self, strategy_id: i64, now_ms: i64) -> Option<PaperState> {
        self.gateway.forget(strategy_id, now_ms);
        self.runners.remove(&strategy_id).map(|r| r.state().clone())
    }

    /// Whether the live-trading master switch is on (never at startup).
    pub fn live_enabled(&self) -> bool {
        self.gateway.live_enabled()
    }

    /// Turns the master switch on (typed phrase, audited first) or off
    /// (disarming all).
    pub fn set_live_enabled(
        &mut self,
        on: bool,
        typed: &str,
        now_ms: i64,
    ) -> Result<(), GatewayError> {
        self.gateway.set_live_enabled(on, typed, now_ms)
    }

    /// Installs the broker that can trade.
    pub fn install_live_route(&mut self, route: Arc<LiveRoute>) {
        self.gateway.install_live_route(route);
    }

    /// The live route, for reconciling its book.
    pub fn live_route(&self) -> Option<&Arc<LiveRoute>> {
        self.gateway.live_route()
    }

    /// The broker disconnected: every strategy is disarmed, and unsent live
    /// orders are dropped (their runners go back to what they hold).
    pub fn remove_live_route(&mut self, now_ms: i64) -> Option<Arc<LiveRoute>> {
        let waiting: Vec<i64> = self
            .gateway
            .take_dispatches()
            .iter()
            .map(LiveDispatch::strategy_id)
            .collect();
        for id in waiting {
            if let Some(runner) = self.runners.get_mut(&id) {
                runner.on_rejected();
            }
        }
        self.gateway.remove_live_route(now_ms)
    }

    /// Arms a running strategy for live trading until the session's end
    /// (see [`OrderGateway::arm`]). It must have its own limits and be
    /// flat. Returns when the arming expires.
    pub fn arm(
        &mut self,
        strategy_id: i64,
        name: &str,
        typed: &str,
        conditions: ArmConditions,
        now_ms: i64,
    ) -> Result<i64, GatewayError> {
        let runner = self.runners.get(&strategy_id).ok_or(ArmError::NotRunning)?;
        let flat = runner.state().position.is_none();
        let has_limits = self.limits.contains_key(&strategy_id);
        self.gateway.arm(
            strategy_id,
            name,
            typed,
            has_limits,
            flat,
            conditions,
            now_ms,
        )
    }

    /// Disarms a strategy at once. Returns whether it was armed.
    pub fn disarm(&mut self, strategy_id: i64, now_ms: i64) -> bool {
        self.gateway
            .disarm(strategy_id, "disarmed by the user", now_ms)
    }

    /// When a strategy's arming expires, if it is armed now.
    pub fn armed_until(&self, strategy_id: i64, now_ms: i64) -> Option<i64> {
        self.gateway.armed_until(strategy_id, now_ms)
    }

    /// What became of a sent live order. A refusal returns the runner to
    /// what it holds and counts toward the rejection storm; an ambiguous or
    /// unaudited outcome pauses the strategy until it is reconciled.
    /// Returns the kill switch's new state if this engaged it.
    pub fn on_live_outcome(
        &mut self,
        strategy_id: i64,
        outcome: &Result<OrderAck, GatewayError>,
        now_ms: i64,
    ) -> Option<KillState> {
        match outcome {
            Ok(_) => None,
            Err(GatewayError::AmbiguousPlacement(tag)) => {
                let reason = format!("order {tag} has an unknown outcome");
                self.gateway.pause(strategy_id, &reason, now_ms);
                None
            }
            // The broker took the order but the log missed its answer: the
            // order exists, so the runner is not reset; the strategy waits
            // for reconciliation.
            Err(GatewayError::AuditFailed(err)) => {
                let reason = format!("a live order's acknowledgement was not audited: {err}");
                self.gateway.pause(strategy_id, &reason, now_ms);
                None
            }
            Err(_) => {
                if let Some(runner) = self.runners.get_mut(&strategy_id) {
                    runner.on_rejected();
                }
                self.observe(Observation::Rejection, now_ms)
            }
        }
    }

    /// Sets (or clears) a strategy's own limits as saved, without an
    /// audit record: for restoring them at start. Changes go through
    /// [`Self::change_strategy_limits`].
    pub fn set_strategy_limits(&mut self, strategy_id: i64, limits: Option<RiskLimits>) {
        match limits {
            Some(l) => self.limits.insert(strategy_id, l),
            None => self.limits.remove(&strategy_id),
        };
    }

    /// The global limits.
    pub fn risk_limits(&self) -> &RiskLimits {
        self.gateway.risk_limits()
    }

    /// A strategy's own limits, if it has any.
    pub fn strategy_limits(&self, strategy_id: i64) -> Option<&RiskLimits> {
        self.limits.get(&strategy_id)
    }

    /// Changes the global limits (see [`OrderGateway::set_risk_limits`]);
    /// the kill switch's daily-loss trigger follows. Returns whether it
    /// loosened.
    pub fn change_risk_limits(
        &mut self,
        limits: RiskLimits,
        typed: &str,
        now_ms: i64,
    ) -> Result<bool, GatewayError> {
        let loss = limits.max_daily_loss_paise;
        let loosened = self.gateway.set_risk_limits(limits, typed, now_ms)?;
        self.monitor.set_max_daily_loss(loss);
        Ok(loosened)
    }

    /// Changes (or, with `None`, removes) a strategy's own limits. Adding
    /// or tightening applies at once; loosening or removing needs the
    /// typed phrase and is audited first. A strategy left without limits
    /// is disarmed. Returns whether it loosened.
    pub fn change_strategy_limits(
        &mut self,
        strategy_id: i64,
        limits: Option<RiskLimits>,
        typed: &str,
        now_ms: i64,
    ) -> Result<bool, GatewayError> {
        let loosened = match (&limits, self.limits.get(&strategy_id)) {
            (Some(new), Some(old)) => !new.is_within(old),
            (Some(_), None) => false,
            (None, Some(_)) => true,
            (None, None) => return Ok(false),
        };
        self.gateway
            .record_limits(Some(strategy_id), loosened, limits.as_ref(), typed, now_ms)?;
        if limits.is_none() {
            self.gateway
                .disarm(strategy_id, "its limits were removed", now_ms);
        }
        self.set_strategy_limits(strategy_id, limits);
        Ok(loosened)
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

    /// The account's usage now, marked at the latest closes.
    pub fn usage(&self, now_ms: i64) -> DeskUsage {
        let mut exposure: HashMap<&str, i64> = HashMap::new();
        for state in self.runners.values().map(PaperRunner::state) {
            if let Some(pos) = &state.position {
                let mark = self
                    .marks
                    .get(&pos.symbol)
                    .copied()
                    .unwrap_or(pos.entry_price);
                *exposure.entry(&pos.symbol).or_default() += paise(mark) * pos.quantity;
            }
        }
        let ist = now_ms + IST_OFFSET_MS;
        let midnight = ist - ist.rem_euclid(DAY_MS) - IST_OFFSET_MS;
        DeskUsage {
            // The account view's P&L is account-wide whatever it is asked.
            day_pnl_paise: self.account_view(0, "").day_pnl_paise,
            open_positions: self
                .runners
                .values()
                .filter(|r| r.state().position.is_some())
                .count(),
            largest_exposure_paise: exposure.values().copied().max().unwrap_or(0),
            orders_today: self.gateway.orders_since(midnight),
            orders_last_minute: self.gateway.orders_since(now_ms - 60_000),
        }
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
    /// `freshness` is the data the bar was built from: an armed strategy
    /// trades only on LIVE.
    pub fn on_closed_bar(
        &mut self,
        strategy_id: i64,
        bar: Candle,
        freshness: Freshness,
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
                freshness,
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
        // 4. The automatic kill-switch triggers: rejections, and the day's
        //    loss marked at this bar.
        let mut kill = None;
        for err in &refused {
            if matches!(err, GatewayError::RiskRejected(_) | GatewayError::Broker(_)) {
                kill = kill.or(self.observe(Observation::Rejection, now_ms));
            }
        }
        let day_pnl = self.account_view(strategy_id, &symbol).day_pnl_paise;
        kill = kill.or(self.observe(Observation::DayPnl(day_pnl), now_ms));

        let runner = self.runners.get(&strategy_id)?;
        Some(StepReport {
            fills,
            placed,
            refused,
            state: runner.state().clone(),
            kill,
            dispatches: self.gateway.take_dispatches(),
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
            0,
        );
        desk
    }

    #[test]
    fn a_signal_is_placed_then_filled_on_the_next_bar() {
        let mut desk = desk_with(7, always_in(1));
        let b0 = bar(0, 100.0, 100.0);
        let r0 = desk.on_closed_bar(7, b0, Freshness::Delayed, b0.t).unwrap();
        assert!(r0.placed.is_some());
        assert!(r0.fills.is_empty());
        assert!(r0.state.position.is_none());

        let b1 = bar(1, 101.0, 102.0);
        let r1 = desk.on_closed_bar(7, b1, Freshness::Delayed, b1.t).unwrap();
        assert_eq!(r1.fills.len(), 1);
        assert_eq!(r1.fills[0].price, 101.0);
        let pos = r1.state.position.unwrap();
        assert_eq!((pos.quantity, pos.entry_price), (1, 101.0));
        assert!(r1.placed.is_none(), "no pyramiding");

        let usage = desk.usage(b1.t);
        assert_eq!(usage.open_positions, 1);
        assert_eq!(usage.orders_today, 0, "the order was yesterday's");
        assert_eq!(desk.usage(b0.t).orders_last_minute, 1);
        // One share marked at the 102 close.
        assert_eq!(usage.largest_exposure_paise, 10_200);
        assert_eq!(usage.day_pnl_paise, 100, "₹1 of MTM");
    }

    #[test]
    fn a_risk_refusal_is_reported_and_the_runner_stays_flat() {
        // 10,000 shares is over the 1,000-share cap.
        let mut desk = desk_with(7, always_in(10_000));
        let b0 = bar(0, 100.0, 100.0);
        let r0 = desk.on_closed_bar(7, b0, Freshness::Delayed, b0.t).unwrap();
        assert!(
            matches!(&r0.refused[..], [GatewayError::RiskRejected(r)] if r.check == RiskCheck::Quantity)
        );
        assert!(r0.placed.is_none());
        let b1 = bar(1, 100.0, 100.0);
        let r1 = desk.on_closed_bar(7, b1, Freshness::Delayed, b1.t).unwrap();
        assert!(r1.fills.is_empty());
        assert!(r1.state.position.is_none());
        assert_eq!(
            r1.refused,
            vec![r0.refused[0].clone()],
            "tries again, refused again"
        );
    }

    #[test]
    fn a_daily_loss_breach_engages_the_kill_switch_and_cancels_open_orders() {
        // Long 100 at 100, then the price halves: a ₹5,000 MTM loss, far
        // over a ₹1,000 daily-loss limit.
        let set = RuleSet {
            exit: Rule::Cmp {
                left: nivesh_strategy::Operand::Close,
                cmp: nivesh_strategy::Comparison::Less,
                right: nivesh_strategy::Operand::Constant(60.0),
            },
            ..always_in(100)
        };
        let audit = Arc::new(crate::MemoryAudit::default());
        let mut desk = PaperDesk::with_broker(
            RiskEngine::new(RiskLimits {
                trading_window: None,
                max_daily_loss_paise: 1_000 * 100,
                ..RiskLimits::default()
            }),
            Arc::new(NextOpen::FREE),
            Arc::clone(&audit) as Arc<dyn AuditSink>,
        );
        desk.start(
            1,
            PaperRunner::new(set, "NSE:RELIANCE", 100_000.0).unwrap(),
            0,
        );
        desk.on_closed_bar(1, bar(0, 100.0, 100.0), Freshness::Delayed, T0);
        let r = desk
            .on_closed_bar(1, bar(1, 100.0, 100.0), Freshness::Delayed, T0 + DAY)
            .unwrap();
        assert!(r.kill.is_none());
        let view = desk.account_view(1, "NSE:RELIANCE");
        assert_eq!((view.position_quantity, view.open_positions), (100, 1));

        let r = desk
            .on_closed_bar(1, bar(2, 60.0, 50.0), Freshness::Delayed, T0 + 2 * DAY)
            .unwrap();
        let view = desk.account_view(1, "NSE:RELIANCE");
        assert_eq!(view.day_pnl_paise, -5_000 * 100, "MTM at the 50 close");
        assert_eq!(view.symbol_exposure_paise, 100 * 50 * 100);
        // The risk check lets the exit through (it reduces the position)...
        assert!(r.refused.is_empty(), "{:?}", r.refused);
        assert!(r.placed.is_some());
        // ...but the breach engages the kill switch, which cancels every
        // open order, the exit included.
        let kill = r.kill.expect("engaged");
        assert_eq!(kill.reason, Some(KillReason::DailyLoss));
        assert!(desk.kill_state().engaged);
        assert_eq!(desk.queued(), 0);
        let kinds = audit.kinds();
        assert!(
            kinds.ends_with(&["kill.engaged", "order.cancelled"]),
            "{kinds:?}"
        );

        // Squaring off is a separate, confirmed action: the position stays,
        // and every new intent is refused.
        let r = desk
            .on_closed_bar(1, bar(3, 51.0, 40.0), Freshness::Delayed, T0 + 3 * DAY)
            .unwrap();
        assert!(r.fills.is_empty());
        assert_eq!(r.refused, vec![GatewayError::KillSwitchEngaged]);
        assert_eq!(desk.account_view(1, "NSE:RELIANCE").position_quantity, 100);
    }

    #[test]
    fn a_storm_of_rejections_engages_the_kill_switch() {
        // Over the 1,000-share cap on every bar: five refusals in a row.
        let mut desk = desk_with(7, always_in(10_000));
        let mut kill = None;
        for i in 0..5 {
            let r = desk
                .on_closed_bar(7, bar(i, 100.0, 100.0), Freshness::Delayed, T0 + i * 1_000)
                .unwrap();
            kill = kill.or(r.kill);
        }
        assert_eq!(
            kill.and_then(|k| k.reason),
            Some(KillReason::RejectionStorm)
        );
    }

    #[test]
    fn the_kill_switch_survives_a_restart_and_resets_only_deliberately() {
        let mut desk = desk_with(7, always_in(1));
        desk.on_closed_bar(7, bar(0, 100.0, 100.0), Freshness::Delayed, T0);
        assert_eq!(desk.queued(), 1);
        assert_eq!(
            desk.engage_kill(KillReason::Manual, "pressed", T0)
                .map(|c| c.len()),
            Some(1)
        );
        let saved = desk.kill_state().clone();

        // A fresh desk (an app restart) restores it engaged.
        let mut desk = desk_with(7, always_in(1));
        desk.restore_kill(saved);
        let r = desk
            .on_closed_bar(7, bar(1, 100.0, 100.0), Freshness::Delayed, T0 + DAY)
            .unwrap();
        assert_eq!(r.refused, vec![GatewayError::KillSwitchEngaged]);
        assert!(matches!(
            desk.reset_kill("reset", T0 + DAY),
            Err(GatewayError::Kill(crate::KillError::WrongPhrase))
        ));
        assert!(matches!(
            desk.reset_kill(crate::RESET_PHRASE, T0 + 60_000),
            Err(GatewayError::Kill(crate::KillError::CoolingDown { .. }))
        ));
        desk.reset_kill(crate::RESET_PHRASE, T0 + DAY).unwrap();
        let r = desk
            .on_closed_bar(7, bar(2, 100.0, 100.0), Freshness::Delayed, T0 + 2 * DAY)
            .unwrap();
        assert!(r.placed.is_some(), "trading again after the reset");
    }

    #[test]
    fn a_failed_audit_blocks_the_reset_but_never_the_engage() {
        let mut desk = PaperDesk::with_broker(
            RiskEngine::new(RiskLimits {
                trading_window: None,
                ..RiskLimits::default()
            }),
            Arc::new(NextOpen::FREE),
            Arc::new(crate::MemoryAudit::failing()),
        );
        assert!(desk.engage_kill(KillReason::Manual, "", T0).is_some());
        assert!(desk.kill_state().engaged);
        assert!(matches!(
            desk.reset_kill(crate::RESET_PHRASE, T0 + DAY),
            Err(GatewayError::AuditFailed(_))
        ));
        assert!(desk.kill_state().engaged, "stays engaged without a record");
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
        let r = desk
            .on_closed_bar(7, bar(0, 100.0, 100.0), Freshness::Delayed, T0)
            .unwrap();
        assert!(
            matches!(&r.refused[..], [GatewayError::RiskRejected(x)] if x.check == RiskCheck::Quantity)
        );
        desk.set_strategy_limits(7, None);
        let r = desk
            .on_closed_bar(7, bar(1, 100.0, 100.0), Freshness::Delayed, T0 + DAY)
            .unwrap();
        assert!(r.placed.is_some());
    }

    #[test]
    fn a_mismatch_pauses_the_strategy_until_it_is_resumed() {
        let mut desk = desk_with(7, always_in(1));
        desk.on_closed_bar(7, bar(0, 100.0, 100.0), Freshness::Delayed, T0);
        assert_eq!(desk.queued(), 1);
        let found = crate::Reconciliation {
            mismatches: vec![crate::Mismatch::UnknownOutcome {
                strategy_id: 7,
                tag: "NV7S1".into(),
            }],
            resolved: vec![],
        };
        assert_eq!(desk.apply_reconciliation(&found, T0), vec![7]);
        assert_eq!(
            desk.apply_reconciliation(&found, T0),
            Vec::<i64>::new(),
            "once"
        );
        assert_eq!(desk.queued(), 0, "its queued order is cancelled");
        assert!(
            desk.paused(7)
                .is_some_and(|r| r.contains("unknown_outcome"))
        );
        let r = desk
            .on_closed_bar(7, bar(1, 100.0, 100.0), Freshness::Delayed, T0 + DAY)
            .unwrap();
        assert!(matches!(&r.refused[..], [GatewayError::StrategyPaused(_)]));
        desk.resume(7, T0 + DAY).unwrap();
        let r = desk
            .on_closed_bar(7, bar(2, 100.0, 100.0), Freshness::Delayed, T0 + 2 * DAY)
            .unwrap();
        assert!(r.placed.is_some());
    }

    #[test]
    fn stopping_drops_the_queued_order() {
        let mut desk = desk_with(7, always_in(1));
        let b0 = bar(0, 100.0, 100.0);
        desk.on_closed_bar(7, b0, Freshness::Delayed, b0.t).unwrap();
        assert_eq!(desk.queued(), 1);
        assert!(desk.stop(7, 0).is_some());
        assert_eq!(desk.queued(), 0);
        assert!(!desk.is_running(7));
        assert!(
            desk.on_closed_bar(7, bar(1, 100.0, 100.0), Freshness::Delayed, T0 + DAY)
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
            last = desk.on_closed_bar(1, b, Freshness::Delayed, b.t);
        }
        let state = last.unwrap().state;
        assert!(state.position.is_none());
        assert_eq!(state.realised_pnl, -90.0);
    }
}
