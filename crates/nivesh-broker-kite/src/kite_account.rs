//! The read-only Kite account: session, holdings, positions, orders, trades
//! and funds, mapped by [`crate::account`]. **Nothing here can trade.**

use async_trait::async_trait;
use nivesh_broker::{
    BrokerAccount, BrokerError, BrokerId, Funds, Holding, Order, Positions, SessionState,
    SessionStatus, Trade,
};
use std::sync::Arc;

use crate::account;
use crate::login::{KEY_ACCESS_TOKEN, next_token_flush_value};
use crate::rest::{KiteRest, RateClass};
use nivesh_secrets::SecretStore;

pub struct KiteAccount {
    rest: Arc<KiteRest>,
    /// The token's home, so `session()` can tell "no token at all" (logged
    /// out) apart from "the broker refused it" (expired).
    secrets: Arc<dyn SecretStore>,
}

impl KiteAccount {
    pub fn new(rest: Arc<KiteRest>, secrets: Arc<dyn SecretStore>) -> Self {
        Self { rest, secrets }
    }
}

#[async_trait]
impl BrokerAccount for KiteAccount {
    fn broker_id(&self) -> BrokerId {
        BrokerId::Kite
    }

    async fn session(&self) -> Result<SessionStatus, BrokerError> {
        if !self.rest.has_token() {
            return match self
                .secrets
                .get(KEY_ACCESS_TOKEN)
                .map_err(KiteAccount::secret_err)?
            {
                Some(_) => Err(BrokerError::Upstream(
                    "token is stored but the client has none; restore the session first".into(),
                )),
                None => Ok(SessionStatus {
                    state: SessionState::LoggedOut,
                    expires_at: None,
                }),
            };
        }
        // The profile call is the cheapest check that the day's token is
        // still alive; a TokenException means the user must log in again.
        match self
            .rest
            .get(RateClass::General, "/user/profile", &[])
            .await
        {
            Ok(_) => Ok(SessionStatus {
                state: SessionState::Connected,
                expires_at: Some(next_token_flush_value()),
            }),
            Err(BrokerError::Unauthorized) => Ok(SessionStatus {
                state: SessionState::Expired,
                expires_at: None,
            }),
            Err(e) => Err(e),
        }
    }

    async fn holdings(&self) -> Result<Vec<Holding>, BrokerError> {
        let data = self
            .rest
            .get(RateClass::General, "/portfolio/holdings", &[])
            .await?;
        account::parse_holdings(&data)
    }

    async fn positions(&self) -> Result<Positions, BrokerError> {
        let data = self
            .rest
            .get(RateClass::General, "/portfolio/positions", &[])
            .await?;
        account::parse_positions(&data)
    }

    async fn orders(&self) -> Result<Vec<Order>, BrokerError> {
        let data = self.rest.get(RateClass::General, "/orders", &[]).await?;
        account::parse_orders(&data)
    }

    async fn trades(&self) -> Result<Vec<Trade>, BrokerError> {
        let data = self.rest.get(RateClass::General, "/trades", &[]).await?;
        account::parse_trades(&data)
    }

    async fn funds(&self) -> Result<Funds, BrokerError> {
        let data = self
            .rest
            .get(RateClass::General, "/user/margins", &[])
            .await?;
        account::parse_funds(&data)
    }
}

impl KiteAccount {
    fn secret_err(e: nivesh_secrets::SecretError) -> BrokerError {
        BrokerError::Upstream(format!("secret store: {e}"))
    }
}
