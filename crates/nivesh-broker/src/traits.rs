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
    BrokerId, BrokerOrderId, Funds, Holding, LoginCallback, LoginState, Order, OrderAck, OrderTag,
    Positions, SessionInfo, SessionStatus, Trade,
};
use nivesh_risk::ApprovedOrder;

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

/// The only trait that trades. It takes an [`ApprovedOrder`], which only the
/// risk engine can mint, **by value**, so an approval is spent once. Only
/// `nivesh-exec::OrderGateway` holds one of these.
///
/// Placement is never retried by an implementation. When it fails with an
/// error that [`BrokerError::placement_ambiguous`] flags, the order may or
/// may not exist: the caller finds out with [`Self::find_by_tag`].
#[async_trait]
pub trait BrokerExecution: BrokerAccount {
    /// Places the order under `tag`. An expired approval is refused before
    /// anything is sent.
    async fn place(&self, order: ApprovedOrder, tag: &OrderTag) -> Result<OrderAck, BrokerError>;

    /// Changes an open order to the newly approved terms.
    async fn modify(
        &self,
        id: &BrokerOrderId,
        order: ApprovedOrder,
    ) -> Result<OrderAck, BrokerError>;

    /// Cancels an open order. Cancelling reduces risk, so it needs no
    /// approval.
    async fn cancel(&self, id: &BrokerOrderId) -> Result<OrderAck, BrokerError>;

    /// Today's order carrying `tag`, if the broker has one.
    async fn find_by_tag(&self, tag: &OrderTag) -> Result<Option<Order>, BrokerError> {
        Ok(self
            .orders()
            .await?
            .into_iter()
            .find(|o| o.tag.as_deref() == Some(tag.as_str())))
    }
}
