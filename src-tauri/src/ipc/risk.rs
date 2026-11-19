//! Wire types for the safety controls: the kill switch.

use serde::Serialize;
use ts_rs::TS;

/// The kill switch as the UI shows it.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct KillStatusDto {
    pub engaged: bool,
    /// `manual`, `daily_loss`, `rejection_storm`, `broker_disconnect`,
    /// `stale_data` or `clock_drift`; `null` when not engaged.
    pub reason: Option<String>,
    /// What tripped it, in plain language.
    pub detail: String,
    /// UTC ms it engaged.
    pub engaged_at: Option<i64>,
    /// UTC ms from which a reset is accepted (the cool-down's end).
    pub resettable_at: Option<i64>,
    /// What the user must type to reset it.
    pub reset_phrase: String,
}

impl From<&nivesh_exec::KillState> for KillStatusDto {
    fn from(s: &nivesh_exec::KillState) -> Self {
        Self {
            engaged: s.engaged,
            reason: s.reason.and_then(|r| {
                serde_json::to_value(r)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
            }),
            detail: s.detail.clone(),
            engaged_at: s.engaged_at_ms,
            resettable_at: s.engaged_at_ms.map(|t| t + nivesh_exec::RESET_COOLDOWN_MS),
            reset_phrase: nivesh_exec::RESET_PHRASE.to_owned(),
        }
    }
}
