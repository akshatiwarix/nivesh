//! Per-provider circuit breaker and the status events the UI shows.

use std::time::Duration;

use chrono::{DateTime, Utc};
use nivesh_core::ProviderId;

use crate::DataError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthConfig {
    /// Consecutive failures that open the breaker.
    pub failure_threshold: u32,
    /// How long an open breaker rejects calls before allowing one probe.
    pub cooldown: Duration,
    /// Every failed probe doubles the cool-down, up to this. A provider that
    /// keeps refusing us (NSE blocking an address) is asked less and less often.
    pub max_cooldown: Duration,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 3,
            cooldown: Duration::from_secs(30),
            max_cooldown: Duration::from_secs(10 * 60),
        }
    }
}

/// What the status bar shows for a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthState {
    /// Working.
    Healthy,
    /// Recent failures, still being tried.
    Degraded,
    /// The breaker is open: calls are skipped until the cool-down ends.
    Down,
}

/// Pushed whenever a provider's health state changes (`provider:status`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderStatus {
    pub provider: ProviderId,
    pub state: HealthState,
    pub consecutive_failures: u32,
    pub last_error: Option<String>,
    pub last_ok: Option<DateTime<Utc>>,
    /// The last failure was the provider refusing us (403 or the like), as
    /// opposed to it being slow or unreachable. The status bar says so.
    pub blocked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Closed,
    Open {
        until: Duration,
    },
    /// One probe call is allowed through; `probing` is true while it runs.
    HalfOpen {
        probing: bool,
    },
}

/// May a call go ahead?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permit {
    Yes,
    /// The breaker was open and this call is the single trial.
    Probe,
    No,
}

#[derive(Debug)]
pub(crate) struct Breaker {
    config: HealthConfig,
    phase: Phase,
    failures: u32,
    last_error: Option<String>,
    last_ok: Option<DateTime<Utc>>,
    blocked: bool,
    /// Failed probes since the last success; each doubles the cool-down.
    reopens: u32,
}

impl Breaker {
    pub fn new(config: HealthConfig) -> Self {
        Self {
            config,
            phase: Phase::Closed,
            failures: 0,
            last_error: None,
            last_ok: None,
            blocked: false,
            reopens: 0,
        }
    }

    pub fn state(&self) -> HealthState {
        match self.phase {
            Phase::Open { .. } | Phase::HalfOpen { .. } => HealthState::Down,
            Phase::Closed if self.failures > 0 => HealthState::Degraded,
            Phase::Closed => HealthState::Healthy,
        }
    }

    pub fn status(&self, provider: ProviderId) -> ProviderStatus {
        ProviderStatus {
            provider,
            state: self.state(),
            consecutive_failures: self.failures,
            last_error: self.last_error.clone(),
            last_ok: self.last_ok,
            blocked: self.blocked,
        }
    }

    /// Asks whether a call may proceed at `now`.
    pub fn permit(&mut self, now: Duration) -> Permit {
        match self.phase {
            Phase::Closed => Permit::Yes,
            Phase::Open { until } if now >= until => {
                self.phase = Phase::HalfOpen { probing: true };
                Permit::Probe
            }
            Phase::Open { .. } | Phase::HalfOpen { probing: true } => Permit::No,
            Phase::HalfOpen { probing: false } => {
                self.phase = Phase::HalfOpen { probing: true };
                Permit::Probe
            }
        }
    }

    /// The provider answered usefully.
    pub fn record_success(&mut self) {
        self.phase = Phase::Closed;
        self.failures = 0;
        self.blocked = false;
        self.reopens = 0;
        self.last_ok = Some(Utc::now());
    }

    /// A call ended without telling us anything about health (it was
    /// `Unsupported`, say). Releases a probe slot.
    pub fn record_neutral(&mut self) {
        if let Phase::HalfOpen { probing: true } = self.phase {
            self.phase = Phase::HalfOpen { probing: false };
        }
    }

    pub fn record_failure(&mut self, now: Duration, error: &DataError) {
        self.failures = self.failures.saturating_add(1);
        self.last_error = Some(error.to_string());
        self.blocked = matches!(error, DataError::Blocked);
        let failed_probe = matches!(self.phase, Phase::HalfOpen { .. });
        if failed_probe || self.failures >= self.config.failure_threshold {
            if failed_probe {
                self.reopens = self.reopens.saturating_add(1);
            }
            let cooldown = self.cooldown_for(error);
            self.phase = Phase::Open {
                until: now + cooldown,
            };
        }
    }

    /// The base cool-down doubled once per failed probe so far (capped), and
    /// never shorter than a provider's own "retry after N".
    fn cooldown_for(&self, error: &DataError) -> Duration {
        let doubled = self
            .config
            .cooldown
            .saturating_mul(1u32 << self.reopens.min(16))
            .min(self.config.max_cooldown.max(self.config.cooldown));
        match error {
            DataError::RateLimited {
                retry_after: Some(wait),
            } => doubled.max(*wait),
            _ => doubled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: fn(u64) -> Duration = Duration::from_secs;

    fn breaker() -> Breaker {
        Breaker::new(HealthConfig {
            failure_threshold: 3,
            cooldown: S(30),
            max_cooldown: S(240),
        })
    }

    #[test]
    fn opens_after_threshold_consecutive_failures() {
        let mut b = breaker();
        let e = DataError::Timeout;
        for n in 1..=2 {
            assert_eq!(b.permit(S(0)), Permit::Yes);
            b.record_failure(S(0), &e);
            assert_eq!(b.state(), HealthState::Degraded, "after {n}");
        }
        b.record_failure(S(0), &e);
        assert_eq!(b.state(), HealthState::Down);
        assert_eq!(b.permit(S(1)), Permit::No);
    }

    #[test]
    fn each_failed_probe_doubles_the_cooldown_up_to_the_cap() {
        let mut b = breaker();
        for _ in 0..3 {
            b.record_failure(S(0), &DataError::Blocked);
        }
        // Opened at t=0 for 30 s.
        assert_eq!(b.permit(S(29)), Permit::No);
        assert_eq!(b.permit(S(30)), Permit::Probe);
        // The probe fails at t=30: next cool-down is 60 s, then 120, then 240.
        b.record_failure(S(30), &DataError::Blocked);
        assert_eq!(b.permit(S(89)), Permit::No);
        assert_eq!(b.permit(S(90)), Permit::Probe);
        b.record_failure(S(90), &DataError::Blocked);
        assert_eq!(b.permit(S(209)), Permit::No);
        assert_eq!(b.permit(S(210)), Permit::Probe);
        b.record_failure(S(210), &DataError::Blocked);
        assert_eq!(b.permit(S(449)), Permit::No);
        assert_eq!(b.permit(S(450)), Permit::Probe);
        // Capped at 240 s from here on.
        b.record_failure(S(450), &DataError::Blocked);
        assert_eq!(b.permit(S(689)), Permit::No);
        assert_eq!(b.permit(S(690)), Permit::Probe);
    }

    #[test]
    fn recovery_resets_the_cooldown_and_the_blocked_flag() {
        let mut b = breaker();
        for _ in 0..3 {
            b.record_failure(S(0), &DataError::Blocked);
        }
        assert!(b.status(ProviderId::Nse).blocked);
        assert_eq!(b.permit(S(30)), Permit::Probe);
        b.record_failure(S(30), &DataError::Blocked); // cool-down now 60 s
        assert_eq!(b.permit(S(90)), Permit::Probe);
        b.record_success();
        let ok = b.status(ProviderId::Nse);
        assert!(!ok.blocked);
        assert_eq!(ok.state, HealthState::Healthy);
        // A fresh outage starts from the base cool-down again.
        for _ in 0..3 {
            b.record_failure(S(100), &DataError::Timeout);
        }
        assert_eq!(b.permit(S(130)), Permit::Probe);
    }

    #[test]
    fn blocked_means_refused_not_slow() {
        let mut b = breaker();
        b.record_failure(S(0), &DataError::Timeout);
        assert!(!b.status(ProviderId::Nse).blocked);
        b.record_failure(S(0), &DataError::Blocked);
        assert!(b.status(ProviderId::Nse).blocked);
        b.record_failure(S(0), &DataError::Timeout);
        assert!(
            !b.status(ProviderId::Nse).blocked,
            "reflects the latest failure"
        );
    }

    #[test]
    fn a_success_resets_the_count() {
        let mut b = breaker();
        b.record_failure(S(0), &DataError::Timeout);
        b.record_failure(S(0), &DataError::Timeout);
        b.record_success();
        assert_eq!(b.state(), HealthState::Healthy);
        b.record_failure(S(0), &DataError::Timeout);
        b.record_failure(S(0), &DataError::Timeout);
        assert_eq!(b.state(), HealthState::Degraded);
    }

    #[test]
    fn half_open_allows_one_probe_and_closes_on_success() {
        let mut b = breaker();
        for _ in 0..3 {
            b.record_failure(S(0), &DataError::Blocked);
        }
        assert_eq!(b.permit(S(29)), Permit::No);
        assert_eq!(b.permit(S(30)), Permit::Probe);
        assert_eq!(b.permit(S(30)), Permit::No, "only one probe at a time");
        b.record_success();
        assert_eq!(b.state(), HealthState::Healthy);
        assert_eq!(b.permit(S(31)), Permit::Yes);
    }

    #[test]
    fn a_failed_probe_reopens_for_a_longer_cooldown() {
        let mut b = breaker();
        for _ in 0..3 {
            b.record_failure(S(0), &DataError::Blocked);
        }
        assert_eq!(b.permit(S(30)), Permit::Probe);
        b.record_failure(S(30), &DataError::Blocked);
        // Not the same 30 s again: the cool-down doubled to 60 s.
        assert_eq!(b.permit(S(60)), Permit::No);
        assert_eq!(b.permit(S(89)), Permit::No);
        assert_eq!(b.permit(S(90)), Permit::Probe);
    }

    #[test]
    fn a_neutral_probe_frees_the_slot() {
        let mut b = breaker();
        for _ in 0..3 {
            b.record_failure(S(0), &DataError::Blocked);
        }
        assert_eq!(b.permit(S(30)), Permit::Probe);
        b.record_neutral();
        assert_eq!(b.permit(S(30)), Permit::Probe);
    }

    #[test]
    fn retry_after_extends_the_cooldown() {
        let mut b = breaker();
        let e = DataError::RateLimited {
            retry_after: Some(S(120)),
        };
        for _ in 0..3 {
            b.record_failure(S(0), &e);
        }
        assert_eq!(b.permit(S(100)), Permit::No);
        assert_eq!(b.permit(S(120)), Permit::Probe);
    }

    #[test]
    fn status_reports_the_last_error() {
        let mut b = breaker();
        b.record_failure(S(0), &DataError::Blocked);
        let s = b.status(ProviderId::Nse);
        assert_eq!(s.consecutive_failures, 1);
        assert_eq!(s.last_error.as_deref(), Some("blocked by the provider"));
    }
}
