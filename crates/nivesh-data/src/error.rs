use std::time::Duration;

use thiserror::Error;

/// Why a provider call failed. The chain uses the variant to decide whether to
/// fall back and whether the failure counts against the provider's health.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DataError {
    /// The provider doesn't offer this capability.
    #[error("not supported by this provider")]
    Unsupported,
    /// The request is outside what the provider can serve, e.g. 1-minute bars
    /// for a range longer than it keeps. The message is fit to show the user.
    #[error("{0}")]
    UnsupportedRange(String),
    #[error("rate limited{}", retry_suffix(*.retry_after))]
    RateLimited { retry_after: Option<Duration> },
    /// The provider refused us (e.g. 403 after a session refresh).
    #[error("blocked by the provider")]
    Blocked,
    #[error("unauthorized")]
    Unauthorized,
    #[error("not found")]
    NotFound,
    #[error("timed out")]
    Timeout,
    #[error("upstream error: {0}")]
    Upstream(String),
    #[error("could not parse response: {0}")]
    Parse(String),
}

fn retry_suffix(retry_after: Option<Duration>) -> String {
    match retry_after {
        Some(d) => format!(" (retry after {}s)", d.as_secs()),
        None => String::new(),
    }
}

impl DataError {
    /// Whether this failure says the provider itself is unhealthy. Asking for
    /// something it doesn't have is not a health problem.
    pub fn counts_against_health(&self) -> bool {
        !matches!(
            self,
            DataError::Unsupported | DataError::UnsupportedRange(_) | DataError::NotFound
        )
    }

    /// Short stable code, for logs and the status bar.
    pub fn code(&self) -> &'static str {
        match self {
            DataError::Unsupported => "unsupported",
            DataError::UnsupportedRange(_) => "unsupported_range",
            DataError::RateLimited { .. } => "rate_limited",
            DataError::Blocked => "blocked",
            DataError::Unauthorized => "unauthorized",
            DataError::NotFound => "not_found",
            DataError::Timeout => "timeout",
            DataError::Upstream(_) => "upstream",
            DataError::Parse(_) => "parse",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_real_failures_count_against_health() {
        let healthy_but_empty = [
            DataError::Unsupported,
            DataError::UnsupportedRange("x".into()),
            DataError::NotFound,
        ];
        for e in healthy_but_empty {
            assert!(!e.counts_against_health(), "{e}");
        }
        let failures = [
            DataError::RateLimited { retry_after: None },
            DataError::Blocked,
            DataError::Unauthorized,
            DataError::Timeout,
            DataError::Upstream("x".into()),
            DataError::Parse("x".into()),
        ];
        for e in failures {
            assert!(e.counts_against_health(), "{e}");
        }
    }

    #[test]
    fn rate_limit_message_mentions_retry_after() {
        let e = DataError::RateLimited {
            retry_after: Some(Duration::from_secs(7)),
        };
        assert_eq!(e.to_string(), "rate limited (retry after 7s)");
        assert_eq!(
            DataError::RateLimited { retry_after: None }.to_string(),
            "rate limited"
        );
    }
}
