//! The safety events the UI listens to: `killswitch:changed` whenever the
//! kill switch engages or resets, and `risk:alert` for anything the user
//! must see (an automatic trip, a refused or ambiguous live order, an armed
//! strategy on data that is not LIVE).

use std::sync::{Arc, OnceLock};

use nivesh_exec::KillState;
use tauri::{AppHandle, Emitter as _};

use crate::ipc::risk::{AlertLevel, KillStatusDto, RiskAlertDto};

pub const KILL_EVENT: &str = "killswitch:changed";
pub const ALERT_EVENT: &str = "risk:alert";

/// Emits once the app is up; before that (and in tests) it only logs.
#[derive(Clone, Default)]
pub struct Notifier(Arc<OnceLock<AppHandle>>);

impl Notifier {
    pub fn attach(&self, app: AppHandle) {
        let _ = self.0.set(app);
    }

    pub fn kill_changed(&self, state: &KillState) {
        if let Some(app) = self.0.get()
            && let Err(err) = app.emit(KILL_EVENT, KillStatusDto::from(state))
        {
            tracing::debug!(%err, "{KILL_EVENT} emit failed");
        }
    }

    pub fn alert(&self, level: AlertLevel, strategy_id: Option<i64>, message: impl Into<String>) {
        let alert = RiskAlertDto {
            level,
            strategy_id,
            message: message.into(),
            at: nivesh_risk::now_ms(),
        };
        tracing::warn!(?level, strategy_id, message = %alert.message, "risk alert");
        if let Some(app) = self.0.get()
            && let Err(err) = app.emit(ALERT_EVENT, alert)
        {
            tracing::debug!(%err, "{ALERT_EVENT} emit failed");
        }
    }
}
