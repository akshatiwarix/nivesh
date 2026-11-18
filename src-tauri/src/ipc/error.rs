use serde::Serialize;
use ts_rs::TS;

/// Stable error codes the UI can branch on (for example to offer a retry).
///
/// Serialised as `SCREAMING_SNAKE_CASE` strings. Never rename a variant: add a
/// new one instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// The request was malformed or failed validation.
    InvalidArgument,
    /// The requested item does not exist.
    NotFound,
    /// A dependency is temporarily unavailable. Worth retrying later.
    Unavailable,
    /// Local storage failed.
    Storage,
    /// An unexpected failure: a bug, or an environment problem.
    Internal,
}

impl ErrorCode {
    /// Whether a caller may reasonably retry the same request unchanged.
    pub const fn is_retryable(self) -> bool {
        matches!(self, ErrorCode::Unavailable)
    }
}

/// The error half of every command's `Result`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct IpcError {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
}

impl IpcError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: code.is_retryable(),
        }
    }

    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidArgument, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }
}

impl From<anyhow::Error> for IpcError {
    /// Unexpected failures are logged here, once, with their full cause chain.
    fn from(err: anyhow::Error) -> Self {
        let message = format!("{err:#}");
        tracing::error!(error = %message, "command failed");
        Self::internal(message)
    }
}

impl From<nivesh_store::StoreError> for IpcError {
    fn from(err: nivesh_store::StoreError) -> Self {
        tracing::error!(%err, "store error");
        Self::new(ErrorCode::Storage, err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialises_with_stable_shape() {
        let err = IpcError::invalid_argument("bad symbol");
        assert_eq!(
            serde_json::to_value(&err).unwrap(),
            serde_json::json!({
                "code": "INVALID_ARGUMENT",
                "message": "bad symbol",
                "retryable": false,
            })
        );
    }

    #[test]
    fn only_unavailable_is_retryable() {
        let codes = [
            ErrorCode::InvalidArgument,
            ErrorCode::NotFound,
            ErrorCode::Unavailable,
            ErrorCode::Storage,
            ErrorCode::Internal,
        ];
        for code in codes {
            assert_eq!(
                IpcError::new(code, "x").retryable,
                code == ErrorCode::Unavailable
            );
        }
    }

    #[test]
    fn anyhow_errors_become_internal_with_context() {
        let err = anyhow::anyhow!("disk full").context("saving layout");
        let ipc = IpcError::from(err);
        assert_eq!(ipc.code, ErrorCode::Internal);
        assert_eq!(ipc.message, "saving layout: disk full");
    }
}
