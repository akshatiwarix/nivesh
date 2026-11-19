//! The kill switch: one control that stops all trading (PLAN.md §11.6).
//!
//! Engaging it is immediate and needs no confirmation: making things safer
//! is one keystroke. While engaged, the gateway refuses every new order and
//! has cancelled everything Nivesh had open; the harness also disarms live
//! strategies. The state is plain data the harness persists, so the switch
//! stays engaged across a restart. Resetting it is deliberate: a typed
//! phrase, checked here, and only after a cool-down.
//!
//! [`KillMonitor`] decides the automatic triggers from observations the
//! harness reports: a daily-loss breach, a storm of rejections, a broker
//! disconnect while live with open orders, stale data on a live strategy,
//! and clock drift. Squaring off positions is *not* part of engaging: that
//! is itself a trade, and a separate, confirmed action.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// What the user must type to reset the switch.
pub const RESET_PHRASE: &str = "RESET KILL SWITCH";

/// How long the switch stays engaged at least, before a reset is accepted.
pub const RESET_COOLDOWN_MS: i64 = 5 * 60_000;

/// Why the switch engaged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KillReason {
    /// The user pressed it.
    Manual,
    DailyLoss,
    RejectionStorm,
    BrokerDisconnect,
    StaleData,
    ClockDrift,
}

/// The persisted state.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct KillState {
    pub engaged: bool,
    pub reason: Option<KillReason>,
    /// What tripped it, in plain language.
    pub detail: String,
    /// UTC ms it engaged.
    pub engaged_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KillError {
    #[error("the kill switch is not engaged")]
    NotEngaged,
    #[error("type \"{RESET_PHRASE}\" exactly to reset the kill switch")]
    WrongPhrase,
    #[error("the kill switch can be reset {remaining_s} s from now (cool-down)")]
    CoolingDown { remaining_s: i64 },
}

/// The switch itself.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct KillSwitch {
    state: KillState,
}

impl KillSwitch {
    /// Restores a persisted state: an engaged switch stays engaged.
    pub fn from_state(state: KillState) -> Self {
        Self { state }
    }

    pub fn state(&self) -> &KillState {
        &self.state
    }

    pub fn is_engaged(&self) -> bool {
        self.state.engaged
    }

    /// Engages the switch. Returns `true` if it was not engaged already (an
    /// already-engaged switch keeps its first reason).
    pub fn engage(&mut self, reason: KillReason, detail: impl Into<String>, now_ms: i64) -> bool {
        if self.state.engaged {
            return false;
        }
        self.state = KillState {
            engaged: true,
            reason: Some(reason),
            detail: detail.into(),
            engaged_at_ms: Some(now_ms),
        };
        true
    }

    /// Resets the switch if `typed` is the phrase and the cool-down has
    /// passed. Whitespace around the phrase is ignored; case is not.
    pub fn reset(&mut self, typed: &str, now_ms: i64) -> Result<(), KillError> {
        if !self.state.engaged {
            return Err(KillError::NotEngaged);
        }
        if typed.trim() != RESET_PHRASE {
            return Err(KillError::WrongPhrase);
        }
        let since = now_ms - self.state.engaged_at_ms.unwrap_or(now_ms);
        if since < RESET_COOLDOWN_MS {
            return Err(KillError::CoolingDown {
                remaining_s: (RESET_COOLDOWN_MS - since + 999) / 1000,
            });
        }
        self.state = KillState::default();
        Ok(())
    }
}

/// When the switch trips on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KillTriggers {
    /// Engage when today's loss (realised plus MTM, paise, positive) reaches
    /// this.
    pub max_daily_loss_paise: i64,
    /// Engage on this many risk or broker rejections...
    pub rejection_storm: usize,
    /// ...within this window.
    pub rejection_window_ms: i64,
    /// Engage when a live strategy's data is older than this.
    pub stale_after_ms: i64,
    /// Engage when the local clock is this far from the exchange's.
    pub max_clock_drift_ms: i64,
}

impl Default for KillTriggers {
    fn default() -> Self {
        Self {
            max_daily_loss_paise: 50_000 * 100,
            rejection_storm: 5,
            rejection_window_ms: 60_000,
            stale_after_ms: 30_000,
            max_clock_drift_ms: 2_000,
        }
    }
}

/// Something the harness saw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Observation {
    /// Today's P&L, realised plus MTM, paise.
    DayPnl(i64),
    /// An order was rejected (by risk or by the broker).
    Rejection,
    /// The broker connection dropped.
    BrokerDisconnected {
        live_strategies: usize,
        open_orders: usize,
    },
    /// The age of the newest data a live strategy is trading on.
    DataAge { age_ms: i64, live: bool },
    /// Local clock minus the exchange's, ms.
    ClockDrift(i64),
}

/// Decides the automatic triggers.
#[derive(Debug, Clone)]
pub struct KillMonitor {
    triggers: KillTriggers,
    rejections: VecDeque<i64>,
}

impl KillMonitor {
    pub fn new(triggers: KillTriggers) -> Self {
        Self {
            triggers,
            rejections: VecDeque::new(),
        }
    }

    pub fn triggers(&self) -> &KillTriggers {
        &self.triggers
    }

    /// The reason to engage, if `seen` calls for it.
    pub fn observe(&mut self, seen: Observation, now_ms: i64) -> Option<(KillReason, String)> {
        let t = &self.triggers;
        match seen {
            Observation::DayPnl(pnl) if pnl <= -t.max_daily_loss_paise => Some((
                KillReason::DailyLoss,
                format!(
                    "today's loss ₹{:.0} reached the ₹{:.0} limit",
                    -pnl as f64 / 100.0,
                    t.max_daily_loss_paise as f64 / 100.0
                ),
            )),
            Observation::Rejection => {
                self.rejections.push_back(now_ms);
                while self
                    .rejections
                    .front()
                    .is_some_and(|at| *at <= now_ms - t.rejection_window_ms)
                {
                    self.rejections.pop_front();
                }
                (self.rejections.len() >= t.rejection_storm).then(|| {
                    (
                        KillReason::RejectionStorm,
                        format!(
                            "{} rejections within {} s",
                            self.rejections.len(),
                            t.rejection_window_ms / 1000
                        ),
                    )
                })
            }
            Observation::BrokerDisconnected {
                live_strategies,
                open_orders,
            } if live_strategies > 0 && open_orders > 0 => Some((
                KillReason::BrokerDisconnect,
                format!("the broker disconnected with {open_orders} open order(s) while live"),
            )),
            Observation::DataAge { age_ms, live: true } if age_ms > t.stale_after_ms => Some((
                KillReason::StaleData,
                format!("live data is {} s old", age_ms / 1000),
            )),
            Observation::ClockDrift(drift) if drift.abs() > t.max_clock_drift_ms => Some((
                KillReason::ClockDrift,
                format!("the clock is {drift} ms off the exchange's"),
            )),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    #[test]
    fn engaging_is_immediate_and_keeps_the_first_reason() {
        let mut k = KillSwitch::default();
        assert!(!k.is_engaged());
        assert!(k.engage(KillReason::Manual, "pressed", NOW));
        assert!(!k.engage(KillReason::DailyLoss, "later", NOW + 1));
        assert_eq!(k.state().reason, Some(KillReason::Manual));
        assert_eq!(k.state().engaged_at_ms, Some(NOW));
    }

    #[test]
    fn reset_needs_the_phrase_and_the_cool_down() {
        let mut k = KillSwitch::default();
        assert_eq!(k.reset(RESET_PHRASE, NOW), Err(KillError::NotEngaged));
        k.engage(KillReason::Manual, "", NOW);
        assert_eq!(
            k.reset("reset kill switch", NOW + RESET_COOLDOWN_MS),
            Err(KillError::WrongPhrase)
        );
        assert_eq!(
            k.reset(RESET_PHRASE, NOW + 60_000),
            Err(KillError::CoolingDown { remaining_s: 240 })
        );
        assert!(k.is_engaged());
        k.reset(&format!("  {RESET_PHRASE} "), NOW + RESET_COOLDOWN_MS)
            .unwrap();
        assert!(!k.is_engaged());
        assert_eq!(k.state(), &KillState::default());
    }

    #[test]
    fn the_state_survives_a_restart() {
        let mut k = KillSwitch::default();
        k.engage(KillReason::RejectionStorm, "5 rejections", NOW);
        let saved = serde_json::to_string(k.state()).unwrap();
        let restored = KillSwitch::from_state(serde_json::from_str(&saved).unwrap());
        assert!(restored.is_engaged());
        assert_eq!(restored.state().reason, Some(KillReason::RejectionStorm));
        assert!(saved.contains(r#""reason":"rejection_storm""#));
    }

    fn monitor() -> KillMonitor {
        KillMonitor::new(KillTriggers::default())
    }

    #[test]
    fn a_daily_loss_breach_trips_it() {
        let mut m = monitor();
        assert!(m.observe(Observation::DayPnl(-49_999 * 100), NOW).is_none());
        let (reason, detail) = m.observe(Observation::DayPnl(-50_000 * 100), NOW).unwrap();
        assert_eq!(reason, KillReason::DailyLoss);
        assert!(detail.contains("₹50000"), "{detail}");
    }

    #[test]
    fn a_storm_of_rejections_trips_it_but_spread_out_ones_do_not() {
        let mut m = monitor();
        for i in 0..4 {
            assert!(m.observe(Observation::Rejection, NOW + i * 1_000).is_none());
        }
        assert_eq!(
            m.observe(Observation::Rejection, NOW + 4_000).map(|x| x.0),
            Some(KillReason::RejectionStorm)
        );
        let mut m = monitor();
        for i in 0..10 {
            assert!(
                m.observe(Observation::Rejection, NOW + i * 20_000)
                    .is_none()
            );
        }
    }

    #[test]
    fn a_disconnect_trips_it_only_while_live_with_open_orders() {
        let mut m = monitor();
        let seen = |live, open| Observation::BrokerDisconnected {
            live_strategies: live,
            open_orders: open,
        };
        assert!(m.observe(seen(0, 3), NOW).is_none(), "paper only");
        assert!(m.observe(seen(1, 0), NOW).is_none(), "nothing open");
        assert_eq!(
            m.observe(seen(1, 2), NOW).map(|x| x.0),
            Some(KillReason::BrokerDisconnect)
        );
    }

    #[test]
    fn stale_live_data_and_clock_drift_trip_it() {
        let mut m = monitor();
        let age = |age_ms, live| Observation::DataAge { age_ms, live };
        assert!(m.observe(age(29_000, true), NOW).is_none());
        assert!(
            m.observe(age(300_000, false), NOW).is_none(),
            "DELAYED paper data is expected to lag"
        );
        assert_eq!(
            m.observe(age(31_000, true), NOW).map(|x| x.0),
            Some(KillReason::StaleData)
        );
        assert!(m.observe(Observation::ClockDrift(1_500), NOW).is_none());
        assert_eq!(
            m.observe(Observation::ClockDrift(-2_500), NOW).map(|x| x.0),
            Some(KillReason::ClockDrift)
        );
    }
}
