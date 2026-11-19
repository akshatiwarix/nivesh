//! Broker account domain types. Money is integer paise, quantities are whole
//! shares, and every type is a plain data carrier — the adapters map broker
//! JSON onto these, and the IPC layer maps them onto wire DTOs.

use std::fmt;

use chrono::{DateTime, Utc};
use nivesh_core::InstrumentId;
use secrecy::SecretString;

use crate::status::OrderStatus;

/// Which broker an adapter speaks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BrokerId {
    Kite,
    /// Simulated fills for paper trading (Part 5).
    Paper,
    /// A labelled demo: scrubbed fixture data and synthetic ticks, so the
    /// broker views can be shown without any broker account.
    Demo,
}

impl BrokerId {
    pub fn code(self) -> &'static str {
        match self {
            Self::Kite => "kite",
            Self::Paper => "paper",
            Self::Demo => "demo",
        }
    }
}

impl fmt::Display for BrokerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// The random nonce a login URL carries; the redirect must echo it back, so
/// a login can only be completed by the app that started it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginState(pub String);

/// What the broker's redirect delivered to the app's loopback listener.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginCallback {
    pub request_token: String,
    pub state: String,
}

/// A finished login: a secret token the UI never sees, and when it dies.
/// Kite's tokens expire at the daily reset, so `expires_at` drives the
/// re-login prompt.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub access_token: SecretString,
    pub expires_at: DateTime<Utc>,
    /// The broker's user id, e.g. a Zerodha client id. Never logged.
    pub user_id: String,
}

/// Where a broker session stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Connected,
    /// The token has passed its expiry; the user must log in again.
    Expired,
    LoggedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionStatus {
    pub state: SessionState,
    /// When the token expires, if the session knows.
    pub expires_at: Option<DateTime<Utc>>,
}

/// One stock the user owns outright (demat), not an intraday position.
#[derive(Debug, Clone, PartialEq)]
pub struct Holding {
    pub id: InstrumentId,
    /// Shares owned.
    pub quantity: i64,
    /// Average cost, paise.
    pub average_price: i64,
    /// Last price, paise, when the broker reports it.
    pub last_price: Option<i64>,
    /// Unrealised P&L, paise, when the broker reports it.
    pub pnl: Option<i64>,
}

/// One net or day position (the same stock can appear in both lists).
#[derive(Debug, Clone, PartialEq)]
pub struct Position {
    pub id: InstrumentId,
    /// Signed: positive long, negative short. Zero means closed.
    pub net_quantity: i64,
    /// Today's traded quantity, signed.
    pub day_quantity: i64,
    /// Average entry price, paise.
    pub average_price: i64,
    pub last_price: Option<i64>,
    /// Mark-to-market P&L, paise, when the broker reports it.
    pub pnl: Option<i64>,
    /// Realised P&L today, paise, when the broker reports it.
    pub realised: Option<i64>,
}

/// Day and net positions, as brokers report them separately.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Positions {
    pub day: Vec<Position>,
    pub net: Vec<Position>,
}

/// Buy or sell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Buy,
    Sell,
}

/// How the order executes. Limit is the default everywhere; market orders
/// arrive in Part 6 behind market protection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderKind {
    Market,
    Limit,
    StopLoss,
    StopLossMarket,
}

/// Which margin pool the order rides on. Cash equity only until Part 6
/// says otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Product {
    /// Cash and carry: full money, delivery.
    Cnc,
    /// Intraday margin product.
    Mis,
    /// Futures and options margin product; never used by cash-only Nivesh
    /// today, but the state machine keeps it expressible.
    Nrml,
}

/// A broker order, as the broker's own book reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct Order {
    /// The broker's id for the order.
    pub id: BrokerOrderId,
    pub id_instrument: InstrumentId,
    pub side: Side,
    pub kind: OrderKind,
    pub product: Product,
    pub quantity: i64,
    pub filled_quantity: i64,
    /// The limit price, paise, when the order has one.
    pub price: Option<i64>,
    /// The average fill price, paise, once fills exist.
    pub average_price: Option<i64>,
    pub status: OrderStatus,
    pub placed_at: Option<DateTime<Utc>>,
    /// The client tag Nivesh sends with its own orders (idempotency); brokers
    /// echo it back. Orders the user placed elsewhere have none.
    pub tag: Option<String>,
}

/// The broker's id for one order.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BrokerOrderId(pub String);

impl fmt::Display for BrokerOrderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One executed trade (a fill). A filled order can have several.
#[derive(Debug, Clone, PartialEq)]
pub struct Trade {
    pub order_id: BrokerOrderId,
    pub id_instrument: InstrumentId,
    pub side: Side,
    pub quantity: i64,
    /// Paise.
    pub price: i64,
    pub traded_at: DateTime<Utc>,
}

/// Equity funds view: what is available, what is held against positions and
/// orders, and the total. Paise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Funds {
    pub available: i64,
    pub used: i64,
    pub total: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broker_ids_have_codes() {
        assert_eq!(BrokerId::Kite.code(), "kite");
        assert_eq!(BrokerId::Paper.to_string(), "paper");
        assert_eq!(BrokerId::Demo.to_string(), "demo");
    }

    #[test]
    fn a_session_token_is_never_in_debug() {
        let info = SessionInfo {
            access_token: secrecy::SecretString::from("token-that-must-not-print"),
            // A fixed time: this crate does not need a clock.
            expires_at: DateTime::parse_from_rfc3339("2026-11-20T00:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
            user_id: "XY1234".into(),
        };
        let text = format!("{info:?}");
        assert!(!text.contains("token-that-must-not-print"));
    }
}
