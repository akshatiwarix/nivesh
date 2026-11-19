//! What can go wrong talking to a broker. Codes the UI reacts to (a session
//! that needs re-login, a rate limit it should wait out) are distinct
//! variants, not strings.

use crate::OrderStatus;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BrokerError {
    /// The access token is missing, expired or was refused: the user must
    /// log in again. Never retry automatically.
    #[error("broker session needs a fresh login")]
    Unauthorized,
    /// The broker throttled us.
    #[error("broker rate limited us")]
    RateLimited { retry_after: Option<Duration> },
    /// The request could not be delivered in time.
    #[error("broker request timed out")]
    Timeout,
    /// The broker answered with something unusable (5xx, bad JSON, an error
    /// payload nothing maps to).
    #[error("broker upstream problem: {0}")]
    Upstream(String),
    /// The request itself was wrong (bad symbol, bad product, missing field).
    #[error("broker refused the request: {0}")]
    InvalidRequest(String),
    /// A broker update tried to move an order into a state it cannot reach.
    #[error("order cannot go from {from:?} to {to:?}")]
    InvalidTransition { from: OrderStatus, to: OrderStatus },
    /// The broker has no such thing (an unknown order id, an empty book).
    #[error("broker has no {0}")]
    NotFound(String),
    /// The adapter (or this build) cannot do that at all — e.g. anything that
    /// trades, until Part 6.
    #[error("not supported by this broker adapter")]
    Unsupported,
}

impl BrokerError {
    /// Whether a failed **order placement** may still have reached the
    /// broker: a timeout or an upstream failure after the request left.
    /// Such an order's fate is unknown. It must be looked up by its client
    /// tag (reconciled), never placed again blindly.
    pub fn placement_ambiguous(&self) -> bool {
        matches!(self, Self::Timeout | Self::Upstream(_))
    }

    /// Whether waiting and trying the same read again can plausibly succeed.
    /// `Unauthorized` never is: the user has to log in again.
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimited { .. } | Self::Timeout | Self::Upstream(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryability_is_about_the_network_not_the_session() {
        assert!(BrokerError::Timeout.retryable());
        assert!(
            BrokerError::RateLimited {
                retry_after: Some(Duration::from_secs(2))
            }
            .retryable()
        );
        assert!(BrokerError::Upstream("502".into()).retryable());
        assert!(!BrokerError::Unauthorized.retryable());
        assert!(!BrokerError::InvalidRequest("bad product".into()).retryable());
        assert!(!BrokerError::Unsupported.retryable());
    }
}
