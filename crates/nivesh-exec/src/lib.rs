//! The order gateway: the **only** path from [`OrderIntent`]s to brokers.
//!
//! An intent is never trusted. The gateway checks it against the risk
//! engine (minting an [`ApprovedOrder`]), then routes it to the paper
//! broker (Part 5) — live routing arrives in Part 6 behind the risk
//! engine, the kill switch and the audit log, and the gateway is where the
//! audit hook will sit.
//!
//! Late intents are refused: the signal bar must be the last closed bar
//! the gateway has seen, so a stale strategy cannot fire into the past.

use nivesh_broker::{BrokerError, Side};
use nivesh_core::Candle;
use nivesh_risk::{ApprovedOrder, RiskContext, RiskEngine};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// The intent strategies emit (re-exported from `nivesh-strategy`'s shape).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderIntent {
    pub symbol: String,
    pub side: Side,
    pub quantity: i64,
    pub signal_price: f64,
    pub signal_bar_t: i64,
    pub stop_loss: Option<f64>,
    pub target: Option<f64>,
    pub trailing_stop: Option<f64>,
}

/// Where a fill landed, at the signal or the touched level.
#[derive(Debug, Clone, PartialEq)]
pub struct Fill {
    pub symbol: String,
    pub side: Side,
    pub quantity: i64,
    /// Rupees.
    pub price: f64,
    /// The bar's time the fill belongs to.
    pub bar_t: i64,
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

/// What the gateway fills against: the paper broker's fill model (Part 5)
/// wraps this trait; the live broker joins in Part 6 taking an
/// `ApprovedOrder`.
pub trait PaperBroker: Send + Sync {
    /// Fills an approved order at `bar`'s open (the model: the next bar
    /// after the signal).
    fn fill(&self, order: &ApprovedOrder, bar: &Candle) -> Result<Fill, BrokerError>;
}

/// The gateway over one instrument's flow.
pub struct OrderGateway {
    engine: RiskEngine,
    paper: Arc<dyn PaperBroker>,
    /// The last closed bar seen per symbol, for staleness checks.
    last_closed: HashMap<String, Candle>,
    /// Timestamps of approved orders, newest first (the risk context's
    /// rate-limiter feed).
    approvals: Vec<i64>,
    /// Today's realised P&L, rupees.
    day_pnl: f64,
    /// How many positions are open (the strategy's ledger reports this).
    open_positions: usize,
}

impl OrderGateway {
    pub fn new(engine: RiskEngine, paper: Arc<dyn PaperBroker>) -> Self {
        Self {
            engine,
            paper,
            last_closed: HashMap::new(),
            approvals: Vec::new(),
            day_pnl: 0.0,
            open_positions: 0,
        }
    }

    /// The harness reports each closed bar as it lands, so the gateway can
    /// refuse intents built on anything older.
    pub fn on_closed_bar(&mut self, symbol: &str, bar: Candle) {
        self.last_closed.insert(symbol.to_owned(), bar);
    }

    /// The strategy ledger updates the gateway's view of positions and P&L.
    pub fn set_account_state(&mut self, open_positions: usize, day_pnl: f64) {
        self.open_positions = open_positions;
        self.day_pnl = day_pnl;
    }

    /// Submits an intent. Late intents are refused before the risk check;
    /// a passed check is immediately routed to the paper broker.
    pub fn submit(&mut self, intent: &OrderIntent, now_ms: i64) -> Result<Fill, GatewayError> {
        let last = self
            .last_closed
            .get(&intent.symbol)
            .ok_or(GatewayError::StaleIntent)?;
        if last.t != intent.signal_bar_t {
            return Err(GatewayError::StaleIntent);
        }
        let recent = self.approvals.clone();
        let ctx = RiskContext {
            symbol: &intent.symbol,
            ltp: intent.signal_price,
            open_positions: self.open_positions,
            recent_orders: &recent,
            day_pnl: self.day_pnl,
            now_ms,
        };
        let price = (intent.side == Side::Buy).then_some(intent.signal_price);
        let approved = self
            .engine
            .check(&intent.symbol, intent.side, intent.quantity, price, &ctx)
            .map_err(|_| GatewayError::RiskRejected)?;
        if !approved.is_valid_at(now_ms) {
            return Err(GatewayError::ApprovalExpired);
        }
        self.approvals.insert(0, now_ms);
        self.approvals.truncate(500);
        // The paper model fills at the signal bar itself (the strategy's
        // decision bar); the next-bar convention lives in the backtester,
        // which feeds the bar it wants filled.
        self.paper.fill(&approved, last).map_err(GatewayError::from)
    }

    /// Count of approvals kept, for tests.
    pub fn approvals(&self) -> usize {
        self.approvals.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FixedPaper {
        fills: Mutex<Vec<Fill>>,
    }

    impl PaperBroker for FixedPaper {
        fn fill(&self, order: &ApprovedOrder, bar: &Candle) -> Result<Fill, BrokerError> {
            let fill = Fill {
                symbol: order.symbol().to_owned(),
                side: order.side(),
                quantity: order.quantity(),
                price: bar.c,
                bar_t: bar.t,
                paper: true,
            };
            self.fills.lock().unwrap().push(fill.clone());
            Ok(fill)
        }
    }

    fn intent(symbol: &str, bar_t: i64, qty: i64) -> OrderIntent {
        OrderIntent {
            symbol: symbol.to_owned(),
            side: Side::Buy,
            quantity: qty,
            signal_price: 1_000.0,
            signal_bar_t: bar_t,
            stop_loss: None,
            target: None,
            trailing_stop: None,
        }
    }

    fn gateway() -> OrderGateway {
        OrderGateway::new(
            nivesh_risk::RiskEngine::new(nivesh_risk::RiskLimits::default()),
            Arc::new(FixedPaper {
                fills: Mutex::new(Vec::new()),
            }),
        )
    }

    fn bar(t: i64) -> Candle {
        Candle {
            t,
            o: 1_000.0,
            h: 1_005.0,
            l: 995.0,
            c: 1_000.0,
            v: 100.0,
        }
    }

    const NOW: i64 = 1_800_000_000_000;

    #[test]
    fn a_fresh_intent_fills_through_the_paper_broker() {
        let mut gw = gateway();
        gw.on_closed_bar("NSE:RELIANCE", bar(NOW));
        let fill = gw.submit(&intent("NSE:RELIANCE", NOW, 1), NOW).unwrap();
        assert!(fill.paper);
        assert_eq!(fill.quantity, 1);
        assert_eq!(gw.approvals(), 1, "the decision is recorded");
    }

    #[test]
    fn a_stale_intent_is_refused_before_the_risk_check() {
        let mut gw = gateway();
        gw.on_closed_bar("NSE:RELIANCE", bar(NOW));
        // The intent's signal bar is one bar older than the last closed.
        let err = gw
            .submit(&intent("NSE:RELIANCE", NOW - 60_000, 1), NOW)
            .unwrap_err();
        assert_eq!(err, GatewayError::StaleIntent);
        assert_eq!(gw.approvals(), 0, "no decision was minted");
    }

    #[test]
    fn a_risk_rejection_never_reaches_the_broker() {
        let mut gw = gateway();
        gw.on_closed_bar("NSE:RELIANCE", bar(NOW));
        // 600 shares at ₹1000 = ₹600k, over the ₹500k cap.
        let err = gw
            .submit(&intent("NSE:RELIANCE", NOW, 600), NOW)
            .unwrap_err();
        assert_eq!(err, GatewayError::RiskRejected);
    }
}
