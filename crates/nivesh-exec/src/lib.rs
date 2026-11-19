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

pub mod desk;

use nivesh_broker::BrokerError;
use nivesh_core::{Candle, Side};
use nivesh_risk::{ApprovedOrder, RiskContext, RiskEngine};
use nivesh_strategy::CostModel;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

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
    #[error("the risk engine refused the order")]
    RiskRejected,
    #[error("the intent is stale: its signal bar is not the last closed bar")]
    StaleIntent,
    #[error("the approval expired; the intent must be re-checked")]
    ApprovalExpired,
    #[error("no broker is routed for this kind of order")]
    NoRoute,
    #[error("the broker refused: {0}")]
    Broker(String),
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
    /// The last closed bar per (strategy, symbol) stream.
    last_closed: HashMap<(i64, String), Candle>,
    /// Approved orders waiting to fill, in approval order.
    queued: Vec<Queued>,
    /// Timestamps of approved orders, newest first (the risk context's
    /// rate-limiter feed).
    approvals: Vec<i64>,
    /// Today's realised P&L across strategies, rupees.
    day_pnl: f64,
    /// How many positions are open across strategies.
    open_positions: usize,
}

impl OrderGateway {
    pub fn new(engine: RiskEngine, paper: Arc<dyn PaperBroker>) -> Self {
        Self {
            engine,
            paper,
            last_closed: HashMap::new(),
            queued: Vec::new(),
            approvals: Vec::new(),
            day_pnl: 0.0,
            open_positions: 0,
        }
    }

    /// A bar closed on a strategy's stream. Orders that strategy queued for
    /// `symbol` fill at this bar's open; the bar then becomes the stream's
    /// last closed bar. A bar no newer than the last one changes nothing.
    ///
    /// Each queued order settles exactly once: filled, or refused by the
    /// paper broker (`Err`) and dropped. Nothing is retried.
    pub fn on_closed_bar(
        &mut self,
        strategy_id: i64,
        symbol: &str,
        bar: Candle,
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
                let terms = self.paper.fill(&q.order, &bar)?;
                Ok(Fill {
                    strategy_id,
                    symbol: symbol.to_owned(),
                    side: q.order.side(),
                    quantity: q.order.quantity(),
                    price: terms.price,
                    charges: terms.charges,
                    bar_t: bar.t,
                    decision_id: q.order.decision_id(),
                    paper: true,
                })
            })
            .collect();
        self.last_closed.insert(key, bar);
        fills
    }

    /// The account's view of positions and P&L, for the risk context.
    pub fn set_account_state(&mut self, open_positions: usize, day_pnl: f64) {
        self.open_positions = open_positions;
        self.day_pnl = day_pnl;
    }

    /// Submits an intent. A late intent is refused before the risk check;
    /// an approved one is queued to fill on the stream's next bar. Returns
    /// the risk decision id.
    pub fn submit(&mut self, intent: &OrderIntent, now_ms: i64) -> Result<u64, GatewayError> {
        let last = self
            .last_closed
            .get(&(intent.strategy_id, intent.symbol.clone()))
            .ok_or(GatewayError::StaleIntent)?;
        if last.t != intent.signal_bar_t {
            return Err(GatewayError::StaleIntent);
        }
        let ctx = RiskContext {
            symbol: &intent.symbol,
            ltp: intent.signal_price,
            open_positions: self.open_positions,
            recent_orders: &self.approvals,
            day_pnl: self.day_pnl,
            now_ms,
        };
        let approved = self
            .engine
            .check(&intent.symbol, intent.side, intent.quantity, None, &ctx)
            .map_err(|_| GatewayError::RiskRejected)?;
        if !approved.is_valid_at(now_ms) {
            return Err(GatewayError::ApprovalExpired);
        }
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
            gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW, 1_000.0))
                .is_empty()
        );
        let decision = gw.submit(&intent(NOW, 1), NOW).unwrap();
        assert_eq!(gw.queued(), 1, "queued, not filled");
        let fills = gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW + MIN, 1_002.0));
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
        gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW, 1_000.0));
        let err = gw.submit(&intent(NOW - MIN, 1), NOW).unwrap_err();
        assert_eq!(err, GatewayError::StaleIntent);
        assert_eq!(gw.approvals(), 0, "no decision was minted");
        // An intent on a stream the gateway never saw is stale too.
        let other = OrderIntent {
            strategy_id: 2,
            ..intent(NOW, 1)
        };
        assert_eq!(gw.submit(&other, NOW), Err(GatewayError::StaleIntent));
    }

    #[test]
    fn a_risk_rejection_queues_nothing() {
        let mut gw = gateway();
        gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW, 1_000.0));
        // 600 shares at ₹1000 = ₹600k, over the ₹500k cap.
        let err = gw.submit(&intent(NOW, 600), NOW).unwrap_err();
        assert_eq!(err, GatewayError::RiskRejected);
        assert_eq!(gw.queued(), 0);
        assert!(
            gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW + MIN, 1_000.0))
                .is_empty()
        );
    }

    #[test]
    fn fills_go_only_to_the_strategy_that_asked() {
        let mut gw = gateway();
        gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW, 1_000.0));
        gw.on_closed_bar(2, "NSE:RELIANCE", bar(NOW, 1_000.0));
        gw.submit(&intent(NOW, 1), NOW).unwrap();
        // Strategy 2's next bar fills nothing of strategy 1's.
        assert!(
            gw.on_closed_bar(2, "NSE:RELIANCE", bar(NOW + MIN, 1_000.0))
                .is_empty()
        );
        assert_eq!(
            gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW + MIN, 1_000.0))
                .len(),
            1
        );
    }

    #[test]
    fn a_replayed_bar_fills_nothing_and_forget_drops_the_queue() {
        let mut gw = gateway();
        gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW, 1_000.0));
        gw.submit(&intent(NOW, 1), NOW).unwrap();
        assert!(
            gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW, 1_000.0))
                .is_empty()
        );
        assert_eq!(gw.queued(), 1, "still waiting for a NEWER bar");
        gw.forget(1);
        assert_eq!(gw.queued(), 0);
        assert!(
            gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW + MIN, 1_000.0))
                .is_empty()
        );
    }

    #[test]
    fn a_bar_without_an_open_fills_nothing() {
        let mut gw = gateway();
        gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW, 1_000.0));
        gw.submit(&intent(NOW, 1), NOW).unwrap();
        let settled = gw.on_closed_bar(1, "NSE:RELIANCE", bar(NOW + MIN, 0.0));
        assert!(matches!(&settled[..], [Err(GatewayError::Broker(_))]));
        assert_eq!(gw.queued(), 0, "dropped, never retried blindly");
    }
}
