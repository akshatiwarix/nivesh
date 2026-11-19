//! Wire types for the broker views: the session status the status bar and
//! the login view show. Secrets are never here — only its state and expiry.

use nivesh_broker::{SessionState, SessionStatus};
use serde::Serialize;
use ts_rs::TS;

/// Where a broker session stands, for the UI. `broker` is `null` until a
/// setup exists at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct BrokerSessionDto {
    /// `kite`, when a broker is set up.
    pub broker: Option<String>,
    /// `connected`, `expired` or `logged_out`.
    pub state: SessionStateDto,
    /// When the day's token dies (the 06:00 IST flush), epoch ms.
    pub expires_at_ms: Option<i64>,
    /// The broker's user id (e.g. a Zerodha client id), for the status bar.
    pub user_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SessionStateDto {
    Connected,
    Expired,
    LoggedOut,
}

impl From<SessionState> for SessionStateDto {
    fn from(state: SessionState) -> Self {
        match state {
            SessionState::Connected => Self::Connected,
            SessionState::Expired => Self::Expired,
            SessionState::LoggedOut => Self::LoggedOut,
        }
    }
}

impl BrokerSessionDto {
    pub fn new(broker: Option<String>, status: SessionStatus, user_id: Option<String>) -> Self {
        Self {
            broker,
            state: status.state.into(),
            expires_at_ms: status.expires_at.map(|t| t.timestamp_millis()),
            user_id,
        }
    }

    pub fn connected(broker: &str, user_id: &str, expires_at_ms: i64) -> Self {
        Self {
            broker: Some(broker.to_owned()),
            state: SessionStateDto::Connected,
            expires_at_ms: Some(expires_at_ms),
            user_id: Some(user_id.to_owned()),
        }
    }

    /// The secret store failed while reading the state: say so without
    /// pretending to be logged out.
    pub fn degraded() -> Self {
        Self {
            broker: Some("kite".into()),
            state: SessionStateDto::Expired,
            expires_at_ms: None,
            user_id: None,
        }
    }
}

/// The login URL a started login flow hands the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct BrokerLoginDto {
    /// Open in the system browser; the backend listens for the redirect.
    pub url: String,
}

// ---- Account data -----------------------------------------------------------

use nivesh_broker::{Funds, Holding, Order, Positions, Trade};

/// One holding of the account. Paise are integers; ms are UTC epoch.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HoldingDto {
    pub exchange: String,
    pub symbol: String,
    pub quantity: i64,
    pub average_price: i64,
    pub last_price: Option<i64>,
    pub pnl: Option<i64>,
}

impl From<&Holding> for HoldingDto {
    fn from(h: &Holding) -> Self {
        Self {
            exchange: h.id.exchange().code().to_owned(),
            symbol: h.id.symbol().to_owned(),
            quantity: h.quantity,
            average_price: h.average_price,
            last_price: h.last_price,
            pnl: h.pnl,
        }
    }
}

/// One position row (day or net list).
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PositionDto {
    pub exchange: String,
    pub symbol: String,
    pub net_quantity: i64,
    pub day_quantity: i64,
    pub average_price: i64,
    pub last_price: Option<i64>,
    pub pnl: Option<i64>,
    pub realised: Option<i64>,
}

impl From<&nivesh_broker::Position> for PositionDto {
    fn from(p: &nivesh_broker::Position) -> Self {
        Self {
            exchange: p.id.exchange().code().to_owned(),
            symbol: p.id.symbol().to_owned(),
            net_quantity: p.net_quantity,
            day_quantity: p.day_quantity,
            average_price: p.average_price,
            last_price: p.last_price,
            pnl: p.pnl,
            realised: p.realised,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PositionsDto {
    pub day: Vec<PositionDto>,
    pub net: Vec<PositionDto>,
}

impl From<&Positions> for PositionsDto {
    fn from(p: &Positions) -> Self {
        Self {
            day: p.day.iter().map(Into::into).collect(),
            net: p.net.iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OrderDto {
    pub order_id: String,
    pub exchange: String,
    pub symbol: String,
    /// `BUY` / `SELL`.
    pub side: String,
    /// `LIMIT`, `MARKET`, `SL`, `SLM`.
    pub kind: String,
    /// `CNC`, `MIS`, `NRML`.
    pub product: String,
    pub quantity: i64,
    pub filled_quantity: i64,
    pub price: Option<i64>,
    pub average_price: Option<i64>,
    /// `PENDING`, `OPEN`, `PARTIALLY_FILLED`, `FILLED`, `CANCELLED`, `REJECTED`.
    pub status: String,
    pub placed_at_ms: Option<i64>,
    pub tag: Option<String>,
}

impl From<&Order> for OrderDto {
    fn from(o: &Order) -> Self {
        let status = match o.status {
            nivesh_broker::OrderStatus::Pending => "PENDING",
            nivesh_broker::OrderStatus::Open => "OPEN",
            nivesh_broker::OrderStatus::PartiallyFilled => "PARTIALLY_FILLED",
            nivesh_broker::OrderStatus::Filled => "FILLED",
            nivesh_broker::OrderStatus::Cancelled => "CANCELLED",
            nivesh_broker::OrderStatus::Rejected => "REJECTED",
        };
        Self {
            order_id: o.id.0.clone(),
            exchange: o.id_instrument.exchange().code().to_owned(),
            symbol: o.id_instrument.symbol().to_owned(),
            side: match o.side {
                nivesh_broker::Side::Buy => "BUY",
                nivesh_broker::Side::Sell => "SELL",
            }
            .to_owned(),
            kind: match o.kind {
                nivesh_broker::OrderKind::Market => "MARKET",
                nivesh_broker::OrderKind::Limit => "LIMIT",
                nivesh_broker::OrderKind::StopLoss => "SL",
                nivesh_broker::OrderKind::StopLossMarket => "SLM",
            }
            .to_owned(),
            product: match o.product {
                nivesh_broker::Product::Cnc => "CNC",
                nivesh_broker::Product::Mis => "MIS",
                nivesh_broker::Product::Nrml => "NRML",
            }
            .to_owned(),
            quantity: o.quantity,
            filled_quantity: o.filled_quantity,
            price: o.price,
            average_price: o.average_price,
            status: status.to_owned(),
            placed_at_ms: o.placed_at.map(|t| t.timestamp_millis()),
            tag: o.tag.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TradeDto {
    pub order_id: String,
    pub exchange: String,
    pub symbol: String,
    pub side: String,
    pub quantity: i64,
    pub price: i64,
    pub traded_at_ms: i64,
}

impl From<&Trade> for TradeDto {
    fn from(t: &Trade) -> Self {
        Self {
            order_id: t.order_id.0.clone(),
            exchange: t.id_instrument.exchange().code().to_owned(),
            symbol: t.id_instrument.symbol().to_owned(),
            side: match t.side {
                nivesh_broker::Side::Buy => "BUY",
                nivesh_broker::Side::Sell => "SELL",
            }
            .to_owned(),
            quantity: t.quantity,
            price: t.price,
            traded_at_ms: t.traded_at.timestamp_millis(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FundsDto {
    pub available: i64,
    pub used: i64,
    pub total: i64,
}

impl From<Funds> for FundsDto {
    fn from(f: Funds) -> Self {
        Self {
            available: f.available,
            used: f.used,
            total: f.total,
        }
    }
}

/// Which broker served the account data (`demo` or `kite`), so every view
/// can label it honestly.
pub type BrokerName = String;
