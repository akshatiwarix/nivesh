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

    pub fn connected(user_id: &str, expires_at_ms: i64) -> Self {
        Self {
            broker: Some("kite".into()),
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
