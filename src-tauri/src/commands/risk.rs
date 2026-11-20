//! Safety-control commands: the kill switch, limits, the master switch and
//! arming.
//!
//! Making things safer is immediate: engaging the kill switch, tightening
//! a limit, disarming, switching live trading off. Making them riskier
//! needs a phrase the user types, which Rust checks: resetting the kill
//! switch, loosening a limit, switching live trading on, arming. Such an
//! action without (or with the wrong) phrase changes nothing and answers
//! `{ done: false, confirm: <phrase> }`.

use std::sync::Arc;

use nivesh_risk::{RiskLimits, paise};
use nivesh_store::Store;
use tauri::State;

use crate::broker::BrokerManager;
use crate::commands::strategy::strategy_named;
use crate::ipc::IpcError;
use crate::ipc::broker::SessionStateDto;
use crate::ipc::risk::{
    KillStatusDto, RiskActionDto, RiskLimitsDto, RiskStatusDto, RiskUsageDto, StrategyRiskDto,
};
use crate::paper::{Confirm, PaperManager, PaperRecord};

type Paper<'a> = State<'a, Arc<PaperManager>>;

#[tauri::command]
pub async fn kill_status(paper: Paper<'_>) -> Result<KillStatusDto, IpcError> {
    Ok(KillStatusDto::from(&paper.kill_state()))
}

/// Engages the kill switch: blocks new orders and cancels open ones.
#[tauri::command]
pub async fn kill_engage(paper: Paper<'_>) -> Result<KillStatusDto, IpcError> {
    paper.engage_kill("engaged by the user")?;
    Ok(KillStatusDto::from(&paper.kill_state()))
}

/// Resets the kill switch: `typed` must be the reset phrase.
#[tauri::command]
pub async fn kill_reset(typed: String, paper: Paper<'_>) -> Result<KillStatusDto, IpcError> {
    paper.reset_kill(&typed)?;
    Ok(KillStatusDto::from(&paper.kill_state()))
}

/// Everything the RISK view shows.
#[tauri::command]
pub async fn risk_status(
    paper: Paper<'_>,
    store: State<'_, Arc<Store>>,
) -> Result<RiskStatusDto, IpcError> {
    let rows = store.strategies()?;
    let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
    let view = paper.risk_view(&ids);
    let strategies = rows
        .iter()
        .zip(view.strategies)
        .map(|(row, s)| {
            let feed = store
                .paper_state(row.id)
                .ok()
                .flatten()
                .map(|(_, json)| PaperRecord::decode(&json).live_feed);
            let mode = if s.paused.is_some() {
                "PAUSED"
            } else if s.armed_until.is_some() {
                "ARMED"
            } else {
                "PAPER"
            };
            StrategyRiskDto {
                name: row.name.clone(),
                running: s.running,
                mode: mode.to_owned(),
                armed_until: s.armed_until,
                paused: s.paused,
                feed: feed.map(|live| if live { "LIVE" } else { "DELAYED" }.to_owned()),
                limits: s.limits.as_ref().map(RiskLimitsDto::from),
            }
        })
        .collect();
    Ok(RiskStatusDto {
        kill: KillStatusDto::from(&view.kill),
        live_enabled: view.live_enabled,
        live_route: view.live_route,
        limits: RiskLimitsDto::from(&view.limits),
        usage: RiskUsageDto::from(view.usage),
        strategies,
    })
}

fn action(confirm: Confirm) -> RiskActionDto {
    match confirm {
        Confirm::Done => RiskActionDto {
            done: true,
            confirm: None,
        },
        Confirm::Needs(phrase) => RiskActionDto {
            done: false,
            confirm: Some(phrase),
        },
    }
}

/// Which limit `RISK SET <FIELD>` changes, and how its value reads.
fn set_field(limits: &mut RiskLimits, field: &str, value: f64) -> Result<(), IpcError> {
    if !value.is_finite() || value <= 0.0 {
        return Err(IpcError::invalid_argument(format!(
            "a limit must be a positive number, not {value}"
        )));
    }
    let whole = || -> Result<u32, IpcError> {
        if value.fract() != 0.0 || value > f64::from(u32::MAX) {
            return Err(IpcError::invalid_argument(format!(
                "{field} takes a whole number, not {value}"
            )));
        }
        // Checked above: a whole number in range.
        Ok(value as u32)
    };
    match field.to_ascii_lowercase().as_str() {
        "qty" | "quantity" => limits.max_order_quantity = i64::from(whole()?),
        "value" => limits.max_order_value_paise = paise(value),
        "exposure" => limits.max_symbol_exposure_paise = paise(value),
        "positions" => limits.max_open_positions = whole()? as usize,
        "opm" => limits.max_orders_per_minute = whole()?,
        "opd" => limits.max_orders_per_day = whole()?,
        "loss" => limits.max_daily_loss_paise = paise(value),
        "band" => {
            if value > 50.0 {
                return Err(IpcError::invalid_argument(
                    "the price band is a percent of the LTP, at most 50",
                ));
            }
            limits.price_band = value / 100.0;
        }
        other => {
            return Err(IpcError::invalid_argument(format!(
                "no limit called {other}; one of QTY VALUE EXPOSURE POSITIONS OPM OPD LOSS BAND"
            )));
        }
    }
    Ok(())
}

/// Changes one limit: the global one, or `strategy`'s own (created from
/// the global ones on first use). Money is rupees, the band percent.
#[tauri::command]
pub async fn risk_set_limit(
    strategy: Option<String>,
    field: String,
    value: f64,
    typed: Option<String>,
    paper: Paper<'_>,
    store: State<'_, Arc<Store>>,
) -> Result<RiskActionDto, IpcError> {
    // Validate before touching anything.
    set_field(&mut RiskLimits::default(), &field, value)?;
    let id = match &strategy {
        Some(name) => Some(strategy_named(&store, name)?.id),
        None => None,
    };
    let confirm = paper.change_limits(
        id,
        |l| {
            // Validated above, so this cannot fail.
            let _ = set_field(l, &field, value);
        },
        typed.as_deref().unwrap_or(""),
    )?;
    Ok(action(confirm))
}

/// Removes a strategy's own limits (a loosening: it needs the phrase).
#[tauri::command]
pub async fn risk_clear_limits(
    strategy: String,
    typed: Option<String>,
    paper: Paper<'_>,
    store: State<'_, Arc<Store>>,
) -> Result<RiskActionDto, IpcError> {
    let id = strategy_named(&store, &strategy)?.id;
    let confirm = paper.clear_strategy_limits(id, typed.as_deref().unwrap_or(""))?;
    Ok(action(confirm))
}

/// The live-trading master switch. On needs the phrase; off is immediate.
#[tauri::command]
pub async fn live_switch(
    on: bool,
    typed: Option<String>,
    paper: Paper<'_>,
) -> Result<RiskActionDto, IpcError> {
    Ok(action(
        paper.set_live_enabled(on, typed.as_deref().unwrap_or(""))?,
    ))
}

/// Arms a running strategy for live trading until 15:30 IST. Every
/// precondition is checked before the phrase is asked for.
#[tauri::command]
pub async fn strategy_arm(
    name: String,
    typed: Option<String>,
    paper: Paper<'_>,
    store: State<'_, Arc<Store>>,
    broker: State<'_, Arc<BrokerManager>>,
) -> Result<RiskActionDto, IpcError> {
    let row = strategy_named(&store, &name)?;
    let session = broker.status().await;
    let connected =
        session.state == SessionStateDto::Connected && session.broker.as_deref() == Some("kite");
    let confirm = paper.arm(row.id, &row.name, typed.as_deref().unwrap_or(""), connected)?;
    Ok(action(confirm))
}

/// Disarms a strategy at once. Returns whether it was armed.
#[tauri::command]
pub async fn strategy_disarm(
    name: String,
    paper: Paper<'_>,
    store: State<'_, Arc<Store>>,
) -> Result<bool, IpcError> {
    let row = strategy_named(&store, &name)?;
    Ok(paper.disarm(row.id))
}

/// Resumes a paused strategy: `RESUME <NAME>`. Returns why it had been
/// paused, or `None` if it was not paused. It stays disarmed.
#[tauri::command]
pub async fn strategy_resume(
    name: String,
    paper: Paper<'_>,
    store: State<'_, Arc<Store>>,
) -> Result<Option<String>, IpcError> {
    let row = strategy_named(&store, &name)?;
    paper.resume(row.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_fields_read_rupees_counts_and_percent() {
        let mut l = RiskLimits::default();
        set_field(&mut l, "LOSS", 2_500.0).unwrap();
        assert_eq!(l.max_daily_loss_paise, 250_000);
        set_field(&mut l, "qty", 25.0).unwrap();
        assert_eq!(l.max_order_quantity, 25);
        set_field(&mut l, "band", 5.0).unwrap();
        assert!((l.price_band - 0.05).abs() < 1e-12);
        assert!(set_field(&mut l, "positions", 2.5).is_err());
        assert!(set_field(&mut l, "loss", -1.0).is_err());
        assert!(set_field(&mut l, "band", 80.0).is_err());
        assert!(set_field(&mut l, "leverage", 2.0).is_err());
    }
}
