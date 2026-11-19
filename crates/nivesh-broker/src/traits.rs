//! The broker traits. Split by capability so read-only access — all Part 4
//! needs — cannot accidentally grow order placement: only
//! [`BrokerExecution`] (Part 5) can trade, and only
//! `nivesh-exec::OrderGateway` will hold one.

use async_trait::async_trait;
use nivesh_core::InstrumentId;
use url::Url;

use crate::BrokerError;
use crate::ticks::{TickMode, TickSubscription};
use crate::types::{
    BrokerId, Funds, Holding, LoginCallback, LoginState, Order, Positions, SessionInfo,
    SessionStatus, Trade,
};

/// The login half of a broker: build the URL the user opens in their browser,
/// then turn the redirect's callback into a session. Implementations store
/// the token where `nivesh-secrets` keeps it.
#[async_trait]
pub trait BrokerAuth: Send + Sync {
    /// The URL to open, carrying `state` so the redirect can be matched back
    /// to this login attempt.
    fn login_url(&self, state: &LoginState) -> Result<Url, BrokerError>;

    /// Completes the login with what the redirect delivered, returning the
    /// session. The adapter verifies `state` matches and refuses mismatches.
    async fn complete_login(&self, callback: LoginCallback) -> Result<SessionInfo, BrokerError>;

    /// Ends the session; the stored token is removed or invalidated.
    async fn logout(&self) -> Result<(), BrokerError>;
}

/// Read-only account data: what the user owns, rides on and was charged.
/// **No method here places, changes or cancels an order.**
#[async_trait]
pub trait BrokerAccount: Send + Sync {
    fn broker_id(&self) -> BrokerId;

    /// Where the session stands right now, checked against the broker (or
    /// the token's expiry) — not a cached answer from startup.
    async fn session(&self) -> Result<SessionStatus, BrokerError>;

    async fn holdings(&self) -> Result<Vec<Holding>, BrokerError>;
    async fn positions(&self) -> Result<Positions, BrokerError>;
    async fn orders(&self) -> Result<Vec<Order>, BrokerError>;
    async fn trades(&self) -> Result<Vec<Trade>, BrokerError>;
    async fn funds(&self) -> Result<Funds, BrokerError>;
}

/// The live market-data half of a broker: reference-counted subscriptions to
/// instruments the views are showing. The `MarketData` chain routes to this
/// when a broker is connected, and provenance becomes LIVE.
#[async_trait]
pub trait MarketStream: Send + Sync {
    async fn subscribe(
        &self,
        ids: &[InstrumentId],
        mode: TickMode,
    ) -> Result<TickSubscription, BrokerError>;
}
