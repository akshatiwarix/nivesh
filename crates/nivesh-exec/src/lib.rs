//! The order gateway: the **only** path from [`OrderIntent`]s to brokers.
//!
//! An intent is never trusted. The gateway checks it against the risk
//! engine (minting an [`ApprovedOrder`]), queues the approval, and fills it
//! through the paper broker when the next bar of the strategy's stream
//! closes, at that bar's open: the backtester's convention, so paper and
//! backtest trade alike. Live routing arrives in Part 6 behind the risk
//! engine, the kill switch and the audit log; the gateway is where the
//! audit hook will sit.
//!
//! Late intents are refused: the signal bar must be the last closed bar
//! the gateway has seen on that strategy's stream, so a stale strategy
//! cannot fire into the past.

pub mod audit;
pub mod desk;

use nivesh_broker::BrokerError;
use nivesh_core::{Candle, Side};
use nivesh_risk::{
    ApprovedOrder, OrderRequest, Product, RiskContext, RiskEngine, RiskLimits, RiskRejection,
};
use nivesh_strategy::CostModel;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

pub use audit::{AuditEvent, AuditSink, MemoryAudit};
pub use desk::{PaperDesk, StepReport};

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
    /// The last closed bar per (strategy, symbol) stream.
    last_closed: HashMap<(i64, String), Candle>,
    /// Approved orders waiting to fill, in approval order.
    queued: Vec<Queued>,
    /// Timestamps of approved orders, newest first (the risk context's
    /// rate-limiter feed).
    approvals: Vec<i64>,
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
            last_closed: HashMap::new(),
            queued: Vec::new(),
            approvals: Vec::new(),
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

    /// Submits an intent. A late intent is refused before the risk check;
    /// an approved one is queued to fill on the stream's next bar. Returns
    /// the risk decision id.
    pub fn submit(
        &mut self,
        intent: &OrderIntent,
        account: &AccountView,
        now_ms: i64,
    ) -> Result<u64, GatewayError> {
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
        // The routed order is on the record before it exists anywhere else.
        audit(AuditEvent::OrderRouted {
            strategy_id: intent.strategy_id,
            decision_id: approved.decision_id(),
            symbol: approved.symbol().to_owned(),
            side: approved.side(),
            quantity: approved.quantity(),
            price: approved.price(),
            route: "paper",
        })?;
        self.approvals.insert(0, now_ms);
        self.approvals.truncate(500);
        let decision_id = approved.decision_id();
        self.queued.push(Queued {
            strategy_id: intent.strategy_id,
            order: approved,
        });
        Ok(decision_id)
    }

    /// Drops a strategy's queued orders and its streams (it stopped).
    pub fn forget(&mut self, strategy_id: i64) {
        self.queued.retain(|q| q.strategy_id != strategy_id);
        self.last_closed.retain(|(id, _), _| *id != strategy_id);
    }

    /// Orders waiting to fill, for tests and the dashboard.
    pub fn queued(&self) -> usize {
        self.queued.len()
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
        gw.forget(1);
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
