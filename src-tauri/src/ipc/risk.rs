//! Wire types for the safety controls: the kill switch, limits, usage,
//! arming and alerts.

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

/// How loud an alert is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum AlertLevel {
    /// Worth seeing: a refusal, a fallback to paper.
    Warn,
    /// Trading stopped or its state is unknown: the kill switch tripped, a
    /// live order's outcome is unknown.
    Critical,
}

/// A `risk:alert` event.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RiskAlertDto {
    pub level: AlertLevel,
    pub strategy_id: Option<i64>,
    pub message: String,
    /// UTC ms.
    pub at: i64,
}

/// A set of limits as the RISK view shows them. Money is rupees.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RiskLimitsDto {
    pub max_order_quantity: i64,
    pub max_order_value: f64,
    pub max_symbol_exposure: f64,
    pub max_open_positions: u32,
    pub max_orders_per_minute: u32,
    pub max_orders_per_day: u32,
    pub max_daily_loss: f64,
    /// Percent of the LTP a limit price may sit from it.
    pub price_band_pct: f64,
    /// `delivery`, `intraday`.
    pub products: Vec<String>,
    /// `09:15–15:30` (IST), or `null` for any time.
    pub trading_window: Option<String>,
}

fn rupees(paise: i64) -> f64 {
    paise as f64 / 100.0
}

impl From<&nivesh_risk::RiskLimits> for RiskLimitsDto {
    fn from(l: &nivesh_risk::RiskLimits) -> Self {
        Self {
            max_order_quantity: l.max_order_quantity,
            max_order_value: rupees(l.max_order_value_paise),
            max_symbol_exposure: rupees(l.max_symbol_exposure_paise),
            max_open_positions: u32::try_from(l.max_open_positions).unwrap_or(u32::MAX),
            max_orders_per_minute: l.max_orders_per_minute,
            max_orders_per_day: l.max_orders_per_day,
            max_daily_loss: rupees(l.max_daily_loss_paise),
            price_band_pct: l.price_band * 100.0,
            products: l.allowed_products.iter().map(|p| p.to_string()).collect(),
            trading_window: l.trading_window.map(|w| {
                format!(
                    "{:02}:{:02}–{:02}:{:02}",
                    w.start_min / 60,
                    w.start_min % 60,
                    w.end_min / 60,
                    w.end_min % 60
                )
            }),
        }
    }
}

/// The account's usage against the global limits. Money is rupees.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RiskUsageDto {
    /// Realised plus MTM; negative is a loss.
    pub day_pnl: f64,
    pub open_positions: u32,
    pub largest_exposure: f64,
    pub orders_today: u32,
    pub orders_last_minute: u32,
}

impl From<nivesh_exec::DeskUsage> for RiskUsageDto {
    fn from(u: nivesh_exec::DeskUsage) -> Self {
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        Self {
            day_pnl: rupees(u.day_pnl_paise),
            open_positions: count(u.open_positions),
            largest_exposure: rupees(u.largest_exposure_paise),
            orders_today: count(u.orders_today),
            orders_last_minute: count(u.orders_last_minute),
        }
    }
}

/// One strategy's safety state.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StrategyRiskDto {
    pub name: String,
    pub running: bool,
    /// `PAPER`, `ARMED` or `PAUSED`.
    pub mode: String,
    /// UTC ms the arming expires, when armed.
    pub armed_until: Option<i64>,
    /// Why it is paused.
    pub paused: Option<String>,
    /// `LIVE` or `DELAYED`, for a strategy that has run.
    pub feed: Option<String>,
    /// Its own limits (applied on top of the global ones); arming needs
    /// them.
    pub limits: Option<RiskLimitsDto>,
}

/// Everything the RISK view shows.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RiskStatusDto {
    pub kill: KillStatusDto,
    /// The live-trading master switch (session-only, off at start).
    pub live_enabled: bool,
    /// Whether a broker that can trade is connected. Without one nothing
    /// can be armed.
    pub live_route: bool,
    pub limits: RiskLimitsDto,
    pub usage: RiskUsageDto,
    pub strategies: Vec<StrategyRiskDto>,
}

/// The answer to an action that may need a typed confirmation. `done`
/// false with `confirm` set: nothing changed; type `confirm` and send the
/// action again with it.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RiskActionDto {
    pub done: bool,
    pub confirm: Option<String>,
}
