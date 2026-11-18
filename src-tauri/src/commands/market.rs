use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use nivesh_core::{MarketCalendar, MarketStatus, Session};
use serde::Serialize;
use ts_rs::TS;

use crate::ipc::IpcError;

static CALENDAR: LazyLock<MarketCalendar> = LazyLock::new(MarketCalendar::bundled);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SessionKind {
    PreOpen,
    Normal,
    Closing,
    Closed,
}

impl From<Session> for SessionKind {
    fn from(session: Session) -> Self {
        match session {
            Session::PreOpen => Self::PreOpen,
            Session::Normal => Self::Normal,
            Session::Closing => Self::Closing,
            Session::Closed => Self::Closed,
        }
    }
}

/// The NSE session right now. Times are UTC epoch milliseconds; the UI
/// formats them in IST.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MarketStatusDto {
    pub session: SessionKind,
    pub holiday: Option<String>,
    pub next_change_utc_ms: i64,
    pub calendar_known: bool,
}

impl From<MarketStatus> for MarketStatusDto {
    fn from(status: MarketStatus) -> Self {
        Self {
            session: status.session.into(),
            holiday: status.holiday,
            next_change_utc_ms: status.next_change_utc.saturating_mul(1000),
            calendar_known: status.calendar_known,
        }
    }
}

#[tauri::command]
pub async fn market_status() -> Result<MarketStatusDto, IpcError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| IpcError::internal(format!("system clock is before 1970: {err}")))?;
    let secs = i64::try_from(now.as_secs())
        .map_err(|err| IpcError::internal(format!("system clock out of range: {err}")))?;
    Ok(CALENDAR.status_at(secs).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dto_serialises_with_stable_shape() {
        // 2026-11-18 10:00 IST = 04:30 UTC.
        let status = MarketCalendar::bundled().status_at(1_794_976_200);
        let json = serde_json::to_value(MarketStatusDto::from(status)).unwrap();
        assert_eq!(json["session"], "NORMAL");
        assert_eq!(json["calendarKnown"], true);
        assert!(json["holiday"].is_null());
        assert!(json["nextChangeUtcMs"].as_i64().unwrap() > 1_794_976_200_000);
    }
}
