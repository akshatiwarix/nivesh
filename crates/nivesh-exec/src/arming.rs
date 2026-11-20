//! Live arming (PLAN.md §11.2): how a strategy's orders may go to a real
//! broker, and for how long.
//!
//! Live is deliberate, per strategy, and expires. Going live needs every
//! one of:
//!
//! - the master switch, which is session-only and off at every start;
//! - the kill switch off and the strategy not paused;
//! - a live route (a broker that can trade) installed in the gateway, and
//!   the broker connected;
//! - the strategy's data streaming LIVE (not DELAYED, EOD or STALE);
//! - the strategy's own limits configured;
//! - the typed phrase `ARM <NAME> LIVE`, checked here, in Rust;
//! - the trading session not over: arming expires at 15:30 IST that day.
//!
//! Nothing here is persisted: every start comes up with the master switch
//! off and nothing armed. Making things safer (disarming, the master
//! switch off) never waits on anything; making them riskier is audited
//! first.

use std::collections::BTreeMap;

use nivesh_core::Freshness;
use nivesh_risk::TradingWindow;

/// IST is UTC+05:30.
const IST_OFFSET_MS: i64 = 19_800_000;
const DAY_MS: i64 = 86_400_000;

/// The phrase that arms `name`: `ARM <NAME> LIVE`, the name upper-cased.
pub fn arm_phrase(name: &str) -> String {
    format!("ARM {} LIVE", name.trim().to_uppercase())
}

/// When arming at `now_ms` expires: the end of that IST day's NSE session.
pub fn session_end_ms(now_ms: i64) -> i64 {
    let ist = now_ms + IST_OFFSET_MS;
    let midnight = ist - ist.rem_euclid(DAY_MS) - IST_OFFSET_MS;
    midnight + i64::from(TradingWindow::NSE_SESSION.end_min) * 60_000
}

/// What the harness knows about the strategy's world when it asks to arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmConditions {
    /// The broker's session is live right now.
    pub broker_connected: bool,
    /// The freshness of the strategy's data stream.
    pub data: Freshness,
}

/// Why arming was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArmError {
    #[error("live trading is switched off")]
    MasterOff,
    #[error("the kill switch is engaged")]
    KillEngaged,
    #[error("the strategy is paused: {0}")]
    Paused(String),
    #[error("the strategy is not running")]
    NotRunning,
    #[error("no broker that can trade is connected")]
    NoLiveRoute,
    #[error("the broker is not connected")]
    BrokerDisconnected,
    #[error("the strategy's data is {}, not LIVE", .0.label())]
    DataNotLive(Freshness),
    #[error("the strategy has no limits of its own")]
    NoLimits,
    #[error(
        "the strategy holds a paper position or has an order waiting; stop it or let it go flat first"
    )]
    NotFlat,
    #[error("the trading session is over; arm again on the next trading day")]
    SessionOver,
    #[error("the confirmation phrase does not match; type {expected}")]
    WrongPhrase { expected: String },
    #[error("the audit log could not be written: {0}")]
    AuditFailed(String),
}

/// The master switch and the armed strategies. Starts off and empty.
#[derive(Debug, Clone, Default)]
pub struct Arming {
    enabled: bool,
    /// Armed strategies and when their arming expires (UTC ms).
    armed: BTreeMap<i64, i64>,
}

impl Arming {
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        if !on {
            self.armed.clear();
        }
    }

    /// When `strategy_id`'s arming expires, if it is armed (expired or not).
    pub fn expires(&self, strategy_id: i64) -> Option<i64> {
        self.armed.get(&strategy_id).copied()
    }

    /// Whether `strategy_id` is armed and its arming has not expired.
    pub fn is_armed(&self, strategy_id: i64, now_ms: i64) -> bool {
        self.enabled && self.expires(strategy_id).is_some_and(|end| now_ms < end)
    }

    /// The armed strategies, for disarming them all.
    pub fn armed(&self) -> Vec<i64> {
        self.armed.keys().copied().collect()
    }

    pub(crate) fn arm(&mut self, strategy_id: i64, expires_ms: i64) {
        self.armed.insert(strategy_id, expires_ms);
    }

    /// Returns whether it was armed.
    pub(crate) fn disarm(&mut self, strategy_id: i64) -> bool {
        self.armed.remove(&strategy_id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 11:15 IST on a weekday.
    const MORNING: i64 = 1_795_150_800_000 + 45 * 60_000;

    #[test]
    fn the_phrase_names_the_strategy_in_capitals() {
        assert_eq!(arm_phrase(" Breakout 20 "), "ARM BREAKOUT 20 LIVE");
    }

    #[test]
    fn arming_ends_at_the_close_of_the_ist_day() {
        let end = session_end_ms(MORNING);
        // 15:30 IST is 10:00 UTC.
        assert_eq!(end.rem_euclid(DAY_MS), 10 * 3_600_000);
        assert!(end > MORNING && end - MORNING < DAY_MS);
        // Just after the close, the same day's end is already past.
        assert_eq!(session_end_ms(end + 1), end);
        // At 23:59 IST it is still the same IST day.
        assert_eq!(session_end_ms(end + 8 * 3_600_000 + 29 * 60_000), end);
    }

    #[test]
    fn arming_counts_only_while_the_master_switch_is_on_and_before_expiry() {
        let mut a = Arming::default();
        assert!(!a.enabled(), "off at every start");
        a.set_enabled(true);
        a.arm(1, MORNING + 1_000);
        assert!(a.is_armed(1, MORNING));
        assert!(!a.is_armed(1, MORNING + 1_000), "expired");
        assert!(!a.is_armed(2, MORNING));
        a.set_enabled(false);
        assert!(a.armed().is_empty(), "the master switch off disarms all");
    }
}
