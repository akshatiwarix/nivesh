//! What the gateway writes to the audit log, and where it writes it.
//!
//! The gateway records each step **before** it takes effect: the intent,
//! the risk decision (with its reason), the order it routes and every fill.
//! If a record cannot be written the step does not happen (fail closed): an
//! unaudited order is never sent. The sink is required to build a gateway;
//! there is no "no audit" mode.

use std::sync::Mutex;

use nivesh_core::Side;
use nivesh_risk::{RiskCheck, RiskRejection};

use crate::kill::KillReason;
use serde::Serialize;

/// One audited event.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "event")]
pub enum AuditEvent {
    /// A strategy asked for an order.
    Intent {
        strategy_id: i64,
        symbol: String,
        side: Side,
        quantity: i64,
        signal_price: f64,
        signal_bar_t: i64,
    },
    /// The intent's signal bar is not its stream's last closed bar.
    Stale { strategy_id: i64, signal_bar_t: i64 },
    /// The risk engine refused it.
    RiskRejected {
        strategy_id: i64,
        symbol: String,
        check: RiskCheck,
        reason: String,
    },
    /// The risk engine approved it and the gateway routed it.
    OrderRouted {
        strategy_id: i64,
        decision_id: u64,
        symbol: String,
        side: Side,
        quantity: i64,
        price: Option<f64>,
        /// `paper` until live routing exists.
        route: &'static str,
    },
    /// The order filled.
    Fill {
        strategy_id: i64,
        decision_id: u64,
        symbol: String,
        side: Side,
        quantity: i64,
        price: f64,
        charges: f64,
        bar_t: i64,
        paper: bool,
    },
    /// The order could not fill and was dropped (never retried).
    FillRefused {
        strategy_id: i64,
        decision_id: u64,
        reason: String,
    },
    /// An intent arrived while the kill switch was engaged.
    KillBlocked { strategy_id: i64 },
    /// The kill switch engaged.
    KillEngaged { reason: KillReason, detail: String },
    /// Engaging it cancelled these open orders (decision ids).
    OrdersCancelled { decision_ids: Vec<u64> },
    /// The kill switch was reset (typed confirmation, after the cool-down).
    KillReset,
    /// A strategy was paused (a reconciliation mismatch, or the user).
    StrategyPaused { strategy_id: i64, reason: String },
    /// A paused strategy was resumed.
    StrategyResumed { strategy_id: i64 },
    /// An intent arrived from a paused strategy.
    PausedBlocked { strategy_id: i64 },
}

impl AuditEvent {
    /// The log's `kind` column.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Intent { .. } => "intent",
            Self::Stale { .. } => "intent.stale",
            Self::RiskRejected { .. } => "risk.rejected",
            Self::OrderRouted { .. } => "order.routed",
            Self::Fill { .. } => "fill",
            Self::FillRefused { .. } => "fill.refused",
            Self::KillBlocked { .. } => "kill.blocked",
            Self::KillEngaged { .. } => "kill.engaged",
            Self::OrdersCancelled { .. } => "order.cancelled",
            Self::KillReset => "kill.reset",
            Self::StrategyPaused { .. } => "strategy.paused",
            Self::StrategyResumed { .. } => "strategy.resumed",
            Self::PausedBlocked { .. } => "strategy.blocked",
        }
    }

    /// The strategy it concerns; account-wide events have none.
    pub fn strategy_id(&self) -> Option<i64> {
        match self {
            Self::Intent { strategy_id, .. }
            | Self::Stale { strategy_id, .. }
            | Self::RiskRejected { strategy_id, .. }
            | Self::OrderRouted { strategy_id, .. }
            | Self::Fill { strategy_id, .. }
            | Self::FillRefused { strategy_id, .. }
            | Self::KillBlocked { strategy_id }
            | Self::StrategyPaused { strategy_id, .. }
            | Self::StrategyResumed { strategy_id }
            | Self::PausedBlocked { strategy_id } => Some(*strategy_id),
            Self::KillEngaged { .. } | Self::OrdersCancelled { .. } | Self::KillReset => None,
        }
    }

    /// The details as JSON text.
    pub fn payload(&self) -> String {
        // Every field is a plain number, string or enum: encoding cannot fail.
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_owned())
    }

    pub(crate) fn rejected(strategy_id: i64, symbol: &str, r: &RiskRejection) -> Self {
        Self::RiskRejected {
            strategy_id,
            symbol: symbol.to_owned(),
            check: r.check,
            reason: r.reason.clone(),
        }
    }
}

/// Where audit events go. `record` returns only once the event is durable.
pub trait AuditSink: Send + Sync {
    fn record(&self, at_ms: i64, event: &AuditEvent) -> Result<(), String>;
}

/// Keeps events in memory: for tests and simulations that have no store.
#[derive(Debug, Default)]
pub struct MemoryAudit {
    events: Mutex<Vec<(i64, AuditEvent)>>,
    /// When set, every write fails (to test failing closed).
    pub fail: bool,
}

impl MemoryAudit {
    pub fn failing() -> Self {
        Self {
            events: Mutex::default(),
            fail: true,
        }
    }

    /// The events so far, oldest first.
    pub fn events(&self) -> Vec<(i64, AuditEvent)> {
        self.events.lock().map(|e| e.clone()).unwrap_or_default()
    }

    pub fn kinds(&self) -> Vec<&'static str> {
        self.events().iter().map(|(_, e)| e.kind()).collect()
    }
}

impl AuditSink for MemoryAudit {
    fn record(&self, at_ms: i64, event: &AuditEvent) -> Result<(), String> {
        if self.fail {
            return Err("the audit log is unavailable".into());
        }
        self.events
            .lock()
            .map_err(|_| "the audit log lock was poisoned".to_owned())?
            .push((at_ms, event.clone()));
        Ok(())
    }
}
