//! The order gateway: the **only** path from [`OrderIntent`]s to brokers.
//!
//! An intent is never trusted. The gateway checks it against the risk
//! engine (minting an [`ApprovedOrder`]), queues the approval, and fills it
//! through the paper broker when the next bar of the strategy's stream
//! closes, at that bar's open: the backtester's convention, so paper and
//! backtest trade alike. An armed strategy's approved orders go instead
//! to the live route as a [`LiveDispatch`] (see [`arming`] and [`live`]):
//! only from LIVE data, only while armed, and audited before they are
//! sent.
//!
//! Late intents are refused: the signal bar must be the last closed bar
//! the gateway has seen on that strategy's stream, so a stale strategy
//! cannot fire into the past.

pub mod arming;
pub mod audit;
pub mod desk;
pub mod kill;
pub mod live;
pub mod reconcile;

use nivesh_broker::{BrokerError, OrderTag};
use nivesh_core::{Candle, Freshness, InstrumentId, Side};
use nivesh_risk::{
    ApprovedOrder, OrderRequest, Product, RiskContext, RiskEngine, RiskLimits, RiskRejection,
};
use nivesh_strategy::CostModel;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

pub use arming::{
    ArmConditions, ArmError, Arming, LIVE_PHRASE, LOOSEN_PHRASE, arm_phrase, session_end_ms,
};
pub use audit::{AuditEvent, AuditSink, MemoryAudit};
pub use desk::{DeskUsage, PaperDesk, StepReport};
pub use kill::{
    KillError, KillMonitor, KillReason, KillState, KillSwitch, KillTriggers, Observation,
    RESET_COOLDOWN_MS, RESET_PHRASE,
};
pub use live::{CancelReport, LiveDispatch, LiveRoute};
pub use reconcile::{
    LiveBook, Mismatch, Reconciliation, TrackedOrder, broker_shares, reconcile, reconcile_with,
};

/// What a strategy asks the gateway for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderIntent {
    /// The strategy that emitted it: its bar stream and its fills are
    /// keyed by this.
    pub strategy_id: i64,
    pub symbol: String,
    pub side: Side,
    pub quantity: i64,
    /// The signal bar's close, rupees.
    pub signal_price: f64,
    /// UTC ms of the signal bar.
    pub signal_bar_t: i64,
    /// How fresh the data the signal came from was. Armed strategies only
    /// trade on LIVE; anything unknown counts as STALE.
    #[serde(default = "unknown_freshness")]
    pub freshness: Freshness,
}

fn unknown_freshness() -> Freshness {
    Freshness::Stale
}

/// A paper fill the gateway reports back to the strategy.
#[derive(Debug, Clone, PartialEq)]
pub struct Fill {
    pub strategy_id: i64,
    pub symbol: String,
    pub side: Side,
    pub quantity: i64,
    /// Rupees, after slippage.
    pub price: f64,
    /// Charges on the fill, rupees.
    pub charges: f64,
    /// UTC ms of the bar whose open filled it.
    pub bar_t: i64,
    /// The risk decision the fill rode on (the audit link).
    pub decision_id: u64,
    /// Paper fills say so; live fills will carry the broker's order id.
    pub paper: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GatewayError {
    #[error("the risk engine refused the order: {0}")]
    RiskRejected(RiskRejection),
    #[error("the intent is stale: its signal bar is not the last closed bar")]
    StaleIntent,
    #[error("the approval expired; the intent must be re-checked")]
    ApprovalExpired,
    #[error("no broker is routed for this kind of order")]
    NoRoute,
    #[error("the broker refused: {0}")]
    Broker(String),
    /// The audit record could not be written, so the step did not happen.
    #[error("the audit log could not be written: {0}")]
    AuditFailed(String),
    /// The kill switch is engaged: no new orders.
    #[error("the kill switch is engaged")]
    KillSwitchEngaged,
    /// The strategy is paused until its books are reconciled or the user
    /// resumes it.
    #[error("the strategy is paused: {0}")]
    StrategyPaused(String),
    /// A kill-switch reset was refused.
    #[error(transparent)]
    Kill(#[from] KillError),
    /// Arming was refused.
    #[error(transparent)]
    Arm(#[from] ArmError),
    /// An armed strategy's intent came from data that is not LIVE.
    #[error("live strategies trade only on LIVE data; this signal came from {} data", .0.label())]
    NotLiveData(Freshness),
    /// The action makes things riskier: it needs the typed phrase.
    #[error("this needs confirmation: type {phrase}")]
    NeedsConfirmation { phrase: String },
    /// The broker may or may not have the order: it is reconciled by its
    /// tag, never placed again.
    #[error("the outcome of order {0} is unknown; it will be reconciled, not retried")]
    AmbiguousPlacement(String),
}

impl From<BrokerError> for GatewayError {
    fn from(e: BrokerError) -> Self {
        Self::Broker(e.to_string())
    }
}

/// A simulated fill: price after slippage, and the charges, rupees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaperFillTerms {
    pub price: f64,
    pub charges: f64,
}

/// The paper broker's fill model: how an approved order fills on `bar`
/// (the first bar after the signal).
pub trait PaperBroker: Send + Sync {
    fn fill(&self, order: &ApprovedOrder, bar: &Candle) -> Result<PaperFillTerms, BrokerError>;
}

/// Fills at the bar's open, with the cost model's slippage and charges:
/// the backtester's fill, so paper and backtest agree.
#[derive(Debug, Clone, Copy, Default)]
pub struct NextOpen {
    pub costs: CostModel,
}

impl NextOpen {
    /// No slippage and no charges.
    pub const FREE: Self = Self {
        costs: CostModel::FREE,
    };
}

impl PaperBroker for NextOpen {
    fn fill(&self, order: &ApprovedOrder, bar: &Candle) -> Result<PaperFillTerms, BrokerError> {
        if bar.o.is_finite() && bar.o > 0.0 {
            let price = self.costs.fill_price(order.side(), bar.o);
            let charges = self
                .costs
                .charges(order.side(), order.quantity(), price)
                .total_rupees();
            Ok(PaperFillTerms { price, charges })
        } else {
            Err(BrokerError::InvalidRequest(format!(
                "no fill: the bar at {} has no usable open",
                bar.t
            )))
        }
    }
}

/// An approved order waiting for its stream's next bar.
struct Queued {
    strategy_id: i64,
    order: ApprovedOrder,
}

/// The gateway.
pub struct OrderGateway {
    engine: RiskEngine,
    paper: Arc<dyn PaperBroker>,
    audit: Arc<dyn AuditSink>,
    kill: KillSwitch,
    /// Paused strategies and why.
    paused: std::collections::BTreeMap<i64, String>,
    /// The last closed bar per (strategy, symbol) stream.
    last_closed: HashMap<(i64, String), Candle>,
    /// Approved orders waiting to fill, in approval order.
    queued: Vec<Queued>,
    /// Timestamps of approved orders, newest first (the risk context's
    /// rate-limiter feed).
    approvals: Vec<i64>,
    /// The master switch and the armed strategies. Never persisted.
    arming: Arming,
    /// The broker that can trade, when one is connected.
    live: Option<Arc<LiveRoute>>,
    /// Live orders approved and waiting for the harness to send.
    dispatches: Vec<LiveDispatch>,
    /// The last client-tag sequence number used.
    tag_seq: u64,
}

/// The account as the risk check sees it for one intent. The harness owns
/// the books, so it builds this; the gateway only checks against it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AccountView {
    /// Shares the intent's strategy holds in its symbol.
    pub position_quantity: i64,
    /// Notional held in the symbol across the account, paise.
    pub symbol_exposure_paise: i64,
    /// Positions open across the account.
    pub open_positions: usize,
    /// Today's P&L, realised plus MTM, paise.
    pub day_pnl_paise: i64,
    /// The strategy's own limits, which can only tighten the global ones.
    pub strategy_limits: Option<RiskLimits>,
}

impl OrderGateway {
    /// The audit sink is required: every intent, decision, routed order and
    /// fill is written to it before it takes effect.
    pub fn new(engine: RiskEngine, paper: Arc<dyn PaperBroker>, audit: Arc<dyn AuditSink>) -> Self {
        Self {
            engine,
            paper,
            audit,
            kill: KillSwitch::default(),
            paused: std::collections::BTreeMap::new(),
            last_closed: HashMap::new(),
            queued: Vec::new(),
            approvals: Vec::new(),
            arming: Arming::default(),
            live: None,
            dispatches: Vec::new(),
            tag_seq: 0,
        }
    }

    /// A bar closed on a strategy's stream. Orders that strategy queued for
    /// `symbol` fill at this bar's open; the bar then becomes the stream's
    /// last closed bar. A bar no newer than the last one changes nothing.
    ///
    /// Each queued order settles exactly once: filled, or refused by the
    /// paper broker (`Err`) and dropped. Nothing is retried. A fill is
    /// audited before it is reported; one that cannot be audited is dropped.
    pub fn on_closed_bar(
        &mut self,
        strategy_id: i64,
        symbol: &str,
        bar: Candle,
        now_ms: i64,
    ) -> Vec<Result<Fill, GatewayError>> {
        let key = (strategy_id, symbol.to_owned());
        if self
            .last_closed
            .get(&key)
            .is_some_and(|last| bar.t <= last.t)
        {
            return Vec::new();
        }
        let (due, rest): (Vec<Queued>, Vec<Queued>) = std::mem::take(&mut self.queued)
            .into_iter()
            .partition(|q| q.strategy_id == strategy_id && q.order.symbol() == symbol);
        self.queued = rest;
        let fills = due
            .into_iter()
            .map(|q| {
                let decision_id = q.order.decision_id();
                let terms = match self.paper.fill(&q.order, &bar) {
                    Ok(terms) => terms,
                    Err(err) => {
                        // Best effort: the order is dropped either way.
                        let _ = self.audit.record(
                            now_ms,
                            &AuditEvent::FillRefused {
                                strategy_id,
                                decision_id,
                                reason: err.to_string(),
                            },
                        );
                        return Err(GatewayError::from(err));
                    }
                };
                let fill = Fill {
                    strategy_id,
                    symbol: symbol.to_owned(),
                    side: q.order.side(),
                    quantity: q.order.quantity(),
                    price: terms.price,
                    charges: terms.charges,
                    bar_t: bar.t,
                    decision_id,
                    paper: true,
                };
                self.audit
                    .record(
                        now_ms,
                        &AuditEvent::Fill {
                            strategy_id,
                            decision_id,
                            symbol: fill.symbol.clone(),
                            side: fill.side,
                            quantity: fill.quantity,
                            price: fill.price,
                            charges: fill.charges,
                            bar_t: fill.bar_t,
                            paper: true,
                        },
                    )
                    .map_err(GatewayError::AuditFailed)?;
                Ok(fill)
            })
            .collect();
        self.last_closed.insert(key, bar);
        fills
    }

    /// The kill switch's state, for the harness to persist.
    pub fn kill_state(&self) -> &KillState {
        self.kill.state()
    }

    /// Restores the persisted kill switch (an engaged one stays engaged).
    pub fn restore_kill(&mut self, state: KillState) {
        self.kill = KillSwitch::from_state(state);
    }

    pub fn risk_limits(&self) -> &RiskLimits {
        &self.engine.limits
    }

    /// Replaces the global limits. Tightening applies at once (audited
    /// best-effort); loosening needs the typed [`LOOSEN_PHRASE`] and is
    /// audited first. Returns whether it loosened.
    pub fn set_risk_limits(
        &mut self,
        limits: RiskLimits,
        typed: &str,
        now_ms: i64,
    ) -> Result<bool, GatewayError> {
        let loosened = !limits.is_within(&self.engine.limits);
        self.record_limits(None, loosened, Some(&limits), typed, now_ms)?;
        self.engine.limits = limits;
        Ok(loosened)
    }

    /// Audits a limit change: a loosening must carry the phrase and be on
    /// the record before it applies; a tightening never waits.
    pub(crate) fn record_limits(
        &self,
        strategy_id: Option<i64>,
        loosened: bool,
        limits: Option<&RiskLimits>,
        typed: &str,
        now_ms: i64,
    ) -> Result<(), GatewayError> {
        let event = AuditEvent::LimitsChanged {
            strategy_id,
            loosened,
            limits: limits.cloned(),
        };
        if !loosened {
            let _ = self.audit.record(now_ms, &event);
            return Ok(());
        }
        if typed != LOOSEN_PHRASE {
            return Err(GatewayError::NeedsConfirmation {
                phrase: LOOSEN_PHRASE.to_owned(),
            });
        }
        self.audit
            .record(now_ms, &event)
            .map_err(GatewayError::AuditFailed)
    }

    /// Engages the kill switch: from now on every intent is refused, and
    /// every queued order is cancelled. Returns the cancelled decision ids,
    /// or `None` if it was already engaged. Engaging never waits on the
    /// audit log: it is recorded best-effort, because stopping trading must
    /// not depend on anything.
    pub fn engage_kill(
        &mut self,
        reason: KillReason,
        detail: &str,
        now_ms: i64,
    ) -> Option<Vec<u64>> {
        if !self.kill.engage(reason, detail, now_ms) {
            return None;
        }
        let _ = self.audit.record(
            now_ms,
            &AuditEvent::KillEngaged {
                reason,
                detail: detail.to_owned(),
            },
        );
        self.disarm_all("the kill switch engaged", now_ms);
        let cancelled: Vec<u64> = std::mem::take(&mut self.queued)
            .into_iter()
            .map(|q| q.order.decision_id())
            .chain(
                std::mem::take(&mut self.dispatches)
                    .into_iter()
                    .map(|d| d.decision_id()),
            )
            .collect();
        if !cancelled.is_empty() {
            let _ = self.audit.record(
                now_ms,
                &AuditEvent::OrdersCancelled {
                    decision_ids: cancelled.clone(),
                },
            );
        }
        Some(cancelled)
    }

    /// Resets the kill switch: the typed phrase, after the cool-down. The
    /// reset is audited first; without the record it does not happen.
    pub fn reset_kill(&mut self, typed: &str, now_ms: i64) -> Result<(), GatewayError> {
        let mut next = self.kill.clone();
        next.reset(typed, now_ms)?;
        self.audit
            .record(now_ms, &AuditEvent::KillReset)
            .map_err(GatewayError::AuditFailed)?;
        self.kill = next;
        Ok(())
    }

    /// Pauses a strategy: its intents are refused (audited) until it is
    /// resumed. Its queued orders are cancelled. Returns `false` if it was
    /// already paused.
    pub fn pause(&mut self, strategy_id: i64, reason: &str, now_ms: i64) -> bool {
        if self.paused.contains_key(&strategy_id) {
            return false;
        }
        // Like the kill switch, pausing never waits on the audit log.
        let _ = self.audit.record(
            now_ms,
            &AuditEvent::StrategyPaused {
                strategy_id,
                reason: reason.to_owned(),
            },
        );
        self.paused.insert(strategy_id, reason.to_owned());
        self.queued.retain(|q| q.strategy_id != strategy_id);
        self.dispatches.retain(|d| d.strategy_id() != strategy_id);
        self.disarm(strategy_id, "the strategy was paused", now_ms);
        true
    }

    /// Resumes a paused strategy; audited first.
    pub fn resume(&mut self, strategy_id: i64, now_ms: i64) -> Result<(), GatewayError> {
        if !self.paused.contains_key(&strategy_id) {
            return Ok(());
        }
        self.audit
            .record(now_ms, &AuditEvent::StrategyResumed { strategy_id })
            .map_err(GatewayError::AuditFailed)?;
        self.paused.remove(&strategy_id);
        Ok(())
    }

    /// Why a strategy is paused, if it is.
    pub fn paused(&self, strategy_id: i64) -> Option<&str> {
        self.paused.get(&strategy_id).map(String::as_str)
    }

    /// Submits an intent. A late intent is refused before the risk check;
    /// an approved one is queued to fill on the stream's next bar. Returns
    /// the risk decision id.
    pub fn submit(
        &mut self,
        intent: &OrderIntent,
        account: &AccountView,
        now_ms: i64,
    ) -> Result<u64, GatewayError> {
        // An arming past the session's end is dropped first: back to paper.
        let live = self.armed_now(intent.strategy_id, now_ms);
        let audit = |event: AuditEvent| {
            self.audit
                .record(now_ms, &event)
                .map_err(GatewayError::AuditFailed)
        };
        audit(AuditEvent::Intent {
            strategy_id: intent.strategy_id,
            symbol: intent.symbol.clone(),
            side: intent.side,
            quantity: intent.quantity,
            signal_price: intent.signal_price,
            signal_bar_t: intent.signal_bar_t,
        })?;
        if self.kill.is_engaged() {
            audit(AuditEvent::KillBlocked {
                strategy_id: intent.strategy_id,
            })?;
            return Err(GatewayError::KillSwitchEngaged);
        }
        if let Some(reason) = self.paused.get(&intent.strategy_id) {
            let reason = reason.clone();
            audit(AuditEvent::PausedBlocked {
                strategy_id: intent.strategy_id,
            })?;
            return Err(GatewayError::StrategyPaused(reason));
        }
        if live && intent.freshness != Freshness::Live {
            audit(AuditEvent::NotLive {
                strategy_id: intent.strategy_id,
                freshness: intent.freshness,
            })?;
            return Err(GatewayError::NotLiveData(intent.freshness));
        }
        let fresh = self
            .last_closed
            .get(&(intent.strategy_id, intent.symbol.clone()))
            .is_some_and(|last| last.t == intent.signal_bar_t);
        if !fresh {
            audit(AuditEvent::Stale {
                strategy_id: intent.strategy_id,
                signal_bar_t: intent.signal_bar_t,
            })?;
            return Err(GatewayError::StaleIntent);
        }
        let ctx = RiskContext {
            ltp: intent.signal_price,
            position_quantity: account.position_quantity,
            symbol_exposure_paise: account.symbol_exposure_paise,
            open_positions: account.open_positions,
            recent_orders: &self.approvals,
            day_pnl_paise: account.day_pnl_paise,
            strategy_limits: account.strategy_limits.as_ref(),
            now_ms,
        };
        let request = OrderRequest {
            symbol: intent.symbol.clone(),
            side: intent.side,
            quantity: intent.quantity,
            price: None,
            product: Product::Delivery,
        };
        let approved = match self.engine.check(&request, &ctx) {
            Ok(approved) => approved,
            Err(rejection) => {
                audit(AuditEvent::rejected(
                    intent.strategy_id,
                    &intent.symbol,
                    &rejection,
                ))?;
                return Err(GatewayError::RiskRejected(rejection));
            }
        };
        if !approved.is_valid_at(now_ms) {
            return Err(GatewayError::ApprovalExpired);
        }
        // A live order needs the route, a parseable instrument and a tag
        // before anything is put on the record.
        let live = if live {
            let route = self.live.clone().ok_or(GatewayError::NoRoute)?;
            let id: InstrumentId = intent.symbol.parse().map_err(|_| GatewayError::NoRoute)?;
            self.tag_seq = (self.tag_seq + 1).max(u64::try_from(now_ms / 1_000).unwrap_or(0));
            let tag = OrderTag::new(intent.strategy_id, self.tag_seq)?;
            Some((route, id, tag))
        } else {
            None
        };
        // The routed order is on the record before it exists anywhere else.
        audit(AuditEvent::OrderRouted {
            strategy_id: intent.strategy_id,
            decision_id: approved.decision_id(),
            symbol: approved.symbol().to_owned(),
            side: approved.side(),
            quantity: approved.quantity(),
            price: approved.price(),
            route: if live.is_some() { "live" } else { "paper" },
        })?;
        self.approvals.insert(0, now_ms);
        self.approvals.truncate(500);
        let decision_id = approved.decision_id();
        match live {
            Some((route, id, tag)) => self.dispatches.push(LiveDispatch::new(
                route,
                intent.strategy_id,
                id,
                approved,
                tag,
            )),
            None => self.queued.push(Queued {
                strategy_id: intent.strategy_id,
                order: approved,
            }),
        }
        Ok(decision_id)
    }

    /// Drops a strategy's queued orders and its streams, and disarms it
    /// (it stopped, or restarted: a restart always comes up in paper).
    pub fn forget(&mut self, strategy_id: i64, now_ms: i64) {
        self.queued.retain(|q| q.strategy_id != strategy_id);
        self.dispatches.retain(|d| d.strategy_id() != strategy_id);
        self.last_closed.retain(|(id, _), _| *id != strategy_id);
        self.disarm(strategy_id, "the strategy stopped", now_ms);
    }

    /// Live orders waiting to be sent. The harness takes them, releases its
    /// lock and awaits [`LiveDispatch::send`] on each.
    pub fn take_dispatches(&mut self) -> Vec<LiveDispatch> {
        std::mem::take(&mut self.dispatches)
    }

    /// Whether the live-trading master switch is on.
    pub fn live_enabled(&self) -> bool {
        self.arming.enabled()
    }

    /// Turns the master switch on (with the typed [`LIVE_PHRASE`], audited
    /// first) or off (at once, and disarming every strategy). `typed` is
    /// ignored when switching off.
    pub fn set_live_enabled(
        &mut self,
        on: bool,
        typed: &str,
        now_ms: i64,
    ) -> Result<(), GatewayError> {
        if on == self.arming.enabled() {
            return Ok(());
        }
        let event = AuditEvent::LiveSwitched { enabled: on };
        if on {
            if typed != LIVE_PHRASE {
                return Err(GatewayError::NeedsConfirmation {
                    phrase: LIVE_PHRASE.to_owned(),
                });
            }
            self.audit
                .record(now_ms, &event)
                .map_err(GatewayError::AuditFailed)?;
        } else {
            self.disarm_all("live trading was switched off", now_ms);
            let _ = self.audit.record(now_ms, &event);
        }
        self.arming.set_enabled(on);
        Ok(())
    }

    /// Installs the broker that can trade (it connected).
    pub fn install_live_route(&mut self, route: Arc<LiveRoute>) {
        self.live = Some(route);
    }

    /// Removes the live route (the broker disconnected or logged out),
    /// disarming every strategy and dropping unsent live orders.
    pub fn remove_live_route(&mut self, now_ms: i64) -> Option<Arc<LiveRoute>> {
        self.disarm_all("the broker disconnected", now_ms);
        self.dispatches.clear();
        self.live.take()
    }

    pub fn live_route(&self) -> Option<&Arc<LiveRoute>> {
        self.live.as_ref()
    }

    /// Arms a strategy for live trading until the session's end. Every
    /// precondition is checked here; the arming is audited before it takes
    /// effect. `has_limits`: the strategy has limits of its own. `flat`:
    /// it holds nothing and waits on nothing (a paper position must not
    /// turn into a live one). Returns when the arming expires.
    #[allow(clippy::too_many_arguments)]
    pub fn arm(
        &mut self,
        strategy_id: i64,
        name: &str,
        typed: &str,
        has_limits: bool,
        flat: bool,
        conditions: ArmConditions,
        now_ms: i64,
    ) -> Result<i64, GatewayError> {
        if !self.arming.enabled() {
            return Err(ArmError::MasterOff.into());
        }
        if self.kill.is_engaged() {
            return Err(ArmError::KillEngaged.into());
        }
        if let Some(reason) = self.paused.get(&strategy_id) {
            return Err(ArmError::Paused(reason.clone()).into());
        }
        if self.live.is_none() {
            return Err(ArmError::NoLiveRoute.into());
        }
        if !conditions.broker_connected {
            return Err(ArmError::BrokerDisconnected.into());
        }
        if conditions.data != Freshness::Live {
            return Err(ArmError::DataNotLive(conditions.data).into());
        }
        if !has_limits {
            return Err(ArmError::NoLimits.into());
        }
        if !flat || self.queued.iter().any(|q| q.strategy_id == strategy_id) {
            return Err(ArmError::NotFlat.into());
        }
        let expires_ms = session_end_ms(now_ms);
        if now_ms >= expires_ms {
            return Err(ArmError::SessionOver.into());
        }
        let expected = arm_phrase(name);
        if typed != expected {
            return Err(ArmError::WrongPhrase { expected }.into());
        }
        self.audit
            .record(
                now_ms,
                &AuditEvent::StrategyArmed {
                    strategy_id,
                    expires_ms,
                },
            )
            .map_err(|e| GatewayError::Arm(ArmError::AuditFailed(e)))?;
        self.arming.arm(strategy_id, expires_ms);
        Ok(expires_ms)
    }

    /// Disarms a strategy at once (the audit record is best-effort: making
    /// things safer never waits). Returns whether it was armed.
    pub fn disarm(&mut self, strategy_id: i64, reason: &str, now_ms: i64) -> bool {
        if !self.arming.disarm(strategy_id) {
            return false;
        }
        let _ = self.audit.record(
            now_ms,
            &AuditEvent::StrategyDisarmed {
                strategy_id,
                reason: reason.to_owned(),
            },
        );
        true
    }

    fn disarm_all(&mut self, reason: &str, now_ms: i64) {
        for strategy_id in self.arming.armed() {
            self.disarm(strategy_id, reason, now_ms);
        }
    }

    /// Whether `strategy_id` is armed right now; an arming past the
    /// session's end is dropped here (back to paper, audited).
    fn armed_now(&mut self, strategy_id: i64, now_ms: i64) -> bool {
        if self.arming.is_armed(strategy_id, now_ms) {
            return true;
        }
        self.disarm(strategy_id, "the trading session ended", now_ms);
        false
    }

    /// When `strategy_id`'s arming expires, if it is armed now.
    pub fn armed_until(&self, strategy_id: i64, now_ms: i64) -> Option<i64> {
        self.arming
            .is_armed(strategy_id, now_ms)
            .then(|| self.arming.expires(strategy_id))
            .flatten()
    }

    /// Orders waiting to fill, for tests and the dashboard.
    pub fn queued(&self) -> usize {
        self.queued.len()
    }

    /// Orders approved at or after `since_ms` (the rate limits' usage).
    pub fn orders_since(&self, since_ms: i64) -> usize {
        self.approvals
            .iter()
            .take_while(|t| **t >= since_ms)
            .count()
    }

    /// Count of approvals kept, for tests.
    pub fn approvals(&self) -> usize {
        self.approvals.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent(bar_t: i64, qty: i64) -> OrderIntent {
        OrderIntent {
            strategy_id: 1,
            symbol: "NSE:RELIANCE".to_owned(),
            side: Side::Buy,
            quantity: qty,
            signal_price: 1_000.0,
            signal_bar_t: bar_t,
            freshness: Freshness::Delayed,
        }
    }

    fn gateway() -> OrderGateway {
        OrderGateway::new(
            RiskEngine::new(nivesh_risk::RiskLimits::default()),
            Arc::new(NextOpen::FREE),
            Arc::new(MemoryAudit::default()),
        )
    }

    fn bar(t: i64, o: f64) -> Candle {
        Candle {
            t,
            o,
            h: o + 5.0,
            l: o - 5.0,
            c: 1_000.0,
            v: 100.0,
        }
    }

    const NOW: i64 = 1_800_000_000_000;
    const MIN: i64 = 60_000;

    #[test]
    fn an_approved_intent_fills_at_the_next_bars_open() {
        let mut gw = gateway();
        assert!(
            gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW, 1_000.0), NOW)
                .is_empty()
        );
        let decision = gw
            .submit(&intent(NOW, 1), &AccountView::default(), NOW)
            .unwrap();
        assert_eq!(gw.queued(), 1, "queued, not filled");
        let fills = gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW + MIN, 1_002.0), NOW);
        assert_eq!(fills.len(), 1);
        let fill = fills[0].as_ref().unwrap();
        assert!(fill.paper);
        assert_eq!(
            (fill.side, fill.quantity, fill.price),
            (Side::Buy, 1, 1_002.0)
        );
        assert_eq!(fill.bar_t, NOW + MIN);
        assert_eq!(fill.decision_id, decision);
        assert_eq!(gw.queued(), 0);
    }

    #[test]
    fn a_stale_intent_is_refused_before_the_risk_check() {
        let mut gw = gateway();
        gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW, 1_000.0), NOW);
        let err = gw
            .submit(&intent(NOW - MIN, 1), &AccountView::default(), NOW)
            .unwrap_err();
        assert_eq!(err, GatewayError::StaleIntent);
        assert_eq!(gw.approvals(), 0, "no decision was minted");
        // An intent on a stream the gateway never saw is stale too.
        let other = OrderIntent {
            strategy_id: 2,
            ..intent(NOW, 1)
        };
        assert_eq!(
            gw.submit(&other, &AccountView::default(), NOW),
            Err(GatewayError::StaleIntent)
        );
    }

    #[test]
    fn a_risk_rejection_queues_nothing() {
        let mut gw = gateway();
        gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW, 1_000.0), NOW);
        // 600 shares at ₹1000 = ₹600k, over the ₹500k cap.
        let err = gw
            .submit(&intent(NOW, 600), &AccountView::default(), NOW)
            .unwrap_err();
        assert!(
            matches!(err, GatewayError::RiskRejected(r) if r.check == nivesh_risk::RiskCheck::OrderValue)
        );
        assert_eq!(gw.queued(), 0);
        assert!(
            gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW + MIN, 1_000.0), NOW)
                .is_empty()
        );
    }

    #[test]
    fn fills_go_only_to_the_strategy_that_asked() {
        let mut gw = gateway();
        gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW, 1_000.0), NOW);
        gw.on_closed_bar(2, "NSE:RELIANCE", bar(NOW, 1_000.0), NOW);
        gw.submit(&intent(NOW, 1), &AccountView::default(), NOW)
            .unwrap();
        // Strategy 2's next bar fills nothing of strategy 1's.
        assert!(
            gw.on_closed_bar(2, "NSE:RELIANCE", bar(NOW + MIN, 1_000.0), NOW)
                .is_empty()
        );
        assert_eq!(
            gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW + MIN, 1_000.0), NOW)
                .len(),
            1
        );
    }

    #[test]
    fn a_replayed_bar_fills_nothing_and_forget_drops_the_queue() {
        let mut gw = gateway();
        gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW, 1_000.0), NOW);
        gw.submit(&intent(NOW, 1), &AccountView::default(), NOW)
            .unwrap();
        assert!(
            gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW, 1_000.0), NOW)
                .is_empty()
        );
        assert_eq!(gw.queued(), 1, "still waiting for a NEWER bar");
        gw.forget(1, NOW);
        assert_eq!(gw.queued(), 0);
        assert!(
            gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW + MIN, 1_000.0), NOW)
                .is_empty()
        );
    }

    fn audited(audit: Arc<MemoryAudit>) -> OrderGateway {
        OrderGateway::new(
            RiskEngine::new(nivesh_risk::RiskLimits::default()),
            Arc::new(NextOpen::FREE),
            audit,
        )
    }

    #[test]
    fn every_step_is_audited_in_order() {
        let log = Arc::new(MemoryAudit::default());
        let mut gw = audited(Arc::clone(&log));
        gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW, 1_000.0), NOW);
        let decision = gw
            .submit(&intent(NOW, 1), &AccountView::default(), NOW)
            .unwrap();
        gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW + MIN, 1_001.0), NOW + MIN);
        assert_eq!(log.kinds(), vec!["intent", "order.routed", "fill"]);
        let events = log.events();
        assert!(matches!(
            &events[1].1,
            AuditEvent::OrderRouted { decision_id, route: "paper", .. } if *decision_id == decision
        ));
        assert!(matches!(&events[2].1, AuditEvent::Fill { price, .. } if *price == 1_001.0));
        assert!(events[2].1.payload().contains(r#""event":"fill""#));
    }

    #[test]
    fn refusals_are_audited_with_their_reason() {
        let log = Arc::new(MemoryAudit::default());
        let mut gw = audited(Arc::clone(&log));
        gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW, 1_000.0), NOW);
        let _ = gw.submit(&intent(NOW, 600), &AccountView::default(), NOW);
        let _ = gw.submit(&intent(NOW - MIN, 1), &AccountView::default(), NOW);
        assert_eq!(
            log.kinds(),
            vec!["intent", "risk.rejected", "intent", "intent.stale"]
        );
        assert!(matches!(
            &log.events()[1].1,
            AuditEvent::RiskRejected { check: nivesh_risk::RiskCheck::OrderValue, reason, .. }
                if reason.contains("per-order cap")
        ));
    }

    #[test]
    fn without_an_audit_record_nothing_is_routed() {
        let mut gw = audited(Arc::new(MemoryAudit::failing()));
        gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW, 1_000.0), NOW);
        let err = gw
            .submit(&intent(NOW, 1), &AccountView::default(), NOW)
            .unwrap_err();
        assert!(matches!(err, GatewayError::AuditFailed(_)));
        assert_eq!(gw.queued(), 0, "fail closed");
        assert_eq!(gw.approvals(), 0);
    }

    #[test]
    fn a_bar_without_an_open_fills_nothing() {
        let mut gw = gateway();
        gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW, 1_000.0), NOW);
        gw.submit(&intent(NOW, 1), &AccountView::default(), NOW)
            .unwrap();
        let settled = gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW + MIN, 0.0), NOW);
        assert!(matches!(&settled[..], [Err(GatewayError::Broker(_))]));
        assert_eq!(gw.queued(), 0, "dropped, never retried blindly");
    }
}
