//! Strategy commands: save/list strategies, run backtests, and start/stop
//! the paper runner. Every surface here is PAPER; nothing places a real
//! order.

use std::sync::Arc;

use nivesh_store::Store;
use nivesh_strategy::{BacktestOptions, RuleSet, Template, TemplateParams, backtest};
use tauri::State;

use crate::ipc::IpcError;
use crate::ipc::strategy::{
    BacktestResultDto, BacktestTradeDto, PaperFillDto, PaperStateDto, StrategyDto,
};
use crate::paper::{PaperManager, PaperRecord};

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Lists every saved strategy with its preview and paper status.
#[tauri::command]
pub async fn strategies(store: State<'_, Arc<Store>>) -> Result<Vec<StrategyDto>, IpcError> {
    let rows = store.strategies()?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let set: RuleSet = serde_json::from_str(&row.json).map_err(|e| {
            IpcError::internal(format!("stored strategy {} is unreadable: {e}", row.id))
        })?;
        let running = store
            .paper_state(row.id)
            .ok()
            .flatten()
            .is_some_and(|(running, _)| running);
        let has_state = store.paper_state(row.id).ok().flatten().is_some();
        out.push(StrategyDto {
            id: row.id,
            name: row.name,
            template: row.template,
            entry: set.entry.describe(),
            running,
            has_state,
            paper_pnl: None,
        });
    }
    Ok(out)
}

/// Saves a strategy built from a template: `STRAT MA 20 50 1 [SL 0.02] [T 0.05]`.
/// Returns the id. The arguments are the command's flat IPC shape; a struct
/// would need its own ts-rs export for one call.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn strategy_save(
    name: String,
    template: String,
    fast: usize,
    slow: usize,
    quantity: i64,
    stop_loss: Option<f64>,
    target: Option<f64>,
    store: State<'_, Arc<Store>>,
) -> Result<i64, IpcError> {
    let template_kind = match template.to_ascii_uppercase().as_str() {
        "MA" | "MA CROSSOVER" | "MA CROSSOVER TEMPLATE" => Template::MaCrossover,
        "RSI" | "RSI MEAN REVERSION" | "RSI MEAN REVERSION TEMPLATE" => Template::RsiReversion,
        "BREAKOUT" | "N-BAR BREAKOUT" | "N-BAR BREAKOUT TEMPLATE" => Template::Breakout,
        other => {
            return Err(IpcError::invalid_argument(format!(
                "unknown template {other:?}; try MA, RSI or BREAKOUT"
            )));
        }
    };
    let params = TemplateParams {
        fast,
        slow,
        quantity,
        stop_loss,
        target,
        trailing_stop: None,
    };
    let set = template_kind.build(&params);
    nivesh_strategy::validate(&set).map_err(|e| IpcError::invalid_argument(e.to_string()))?;
    let json = serde_json::to_string(&set)
        .map_err(|e| IpcError::internal(format!("cannot encode the strategy: {e}")))?;
    Ok(store.save_strategy(None, &name, template_kind.title(), &json, now_ms())?)
}

/// Deletes a saved strategy by name.
#[tauri::command]
pub async fn strategy_delete(name: String, store: State<'_, Arc<Store>>) -> Result<(), IpcError> {
    let rows = store.strategies()?;
    if let Some(row) = rows.iter().find(|r| r.name.eq_ignore_ascii_case(&name)) {
        store.delete_strategy(row.id)?;
    } else {
        return Err(IpcError::invalid_argument(format!(
            "no strategy named {name:?}"
        )));
    }
    Ok(())
}

/// Runs a backtest for a strategy over `interval`/`range` and returns the
/// result, saved into the store.
#[tauri::command]
pub async fn backtest_strategy(
    name: String,
    interval: String,
    range: String,
    services: State<'_, Arc<crate::services::Services>>,
    store: State<'_, Arc<Store>>,
) -> Result<BacktestResultDto, IpcError> {
    let rows = store.strategies()?;
    let row = rows
        .iter()
        .find(|r| r.name.eq_ignore_ascii_case(&name))
        .ok_or_else(|| IpcError::invalid_argument(format!("no strategy named {name:?}")))?;
    let set: RuleSet = serde_json::from_str(&row.json)
        .map_err(|e| IpcError::internal(format!("stored strategy is unreadable: {e}")))?;

    // The candles come from the same cached history the chart uses.
    // v1: backtests run on one liquid symbol.
    let series = services
        .market
        .history(&crate::commands::chart::parse_request(
            "NSE",
            "RELIANCE",
            &interval,
            Some(&range),
        )?)
        .await?;
    let candles: Vec<nivesh_core::Candle> = series.data.candles.clone();

    let run_id = store.open_backtest(row.id, "NSE:RELIANCE", &interval, &range, now_ms())?;
    let mut progress = |_, _| true;
    let result = backtest(set, &candles, BacktestOptions::default(), &mut progress);
    let trades_json = serde_json::to_string(&result.trades)
        .map_err(|e| IpcError::internal(format!("cannot encode the trades: {e}")))?;
    store.finish_backtest(
        run_id,
        now_ms(),
        result.total_pnl,
        result.max_drawdown,
        &trades_json,
    )?;

    Ok(BacktestResultDto {
        total_pnl: result.total_pnl,
        total_charges: result.total_charges,
        max_drawdown: result.max_drawdown,
        trades: result
            .trades
            .iter()
            .map(|t| BacktestTradeDto {
                entry_price: t.entry_price,
                exit_price: t.exit_price,
                quantity: t.quantity,
                charges: t.charges,
                pnl: t.pnl,
                entry_t: t.entry_t,
                exit_t: t.exit_t,
            })
            .collect(),
        bars_in_market: result.bars_in_market,
        total_bars: result.total_bars,
    })
}

/// The latest saved backtest result of a strategy.
#[tauri::command]
pub async fn backtest_result(
    name: String,
    store: State<'_, Arc<Store>>,
) -> Result<BacktestResultDto, IpcError> {
    let rows = store.strategies()?;
    let row = rows
        .iter()
        .find(|r| r.name.eq_ignore_ascii_case(&name))
        .ok_or_else(|| IpcError::invalid_argument(format!("no strategy named {name:?}")))?;
    // The newest finished run of this strategy.
    let conn = store.connection()?;
    let (total_pnl, max_drawdown, trades_json): (f64, f64, String) = conn
        .query_row(
            "SELECT total_pnl, max_drawdown, trades_json FROM backtests
             WHERE strategy_id = ?1 AND finished_at IS NOT NULL
             ORDER BY id DESC LIMIT 1",
            (row.id,),
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| IpcError::invalid_argument(format!("no backtest yet for {name:?}")))?;
    let trades: Vec<nivesh_strategy::BacktestTrade> = serde_json::from_str(&trades_json)
        .map_err(|e| IpcError::internal(format!("stored trades are unreadable: {e}")))?;
    Ok(BacktestResultDto {
        total_pnl,
        // Runs saved before the cost model have no charges recorded.
        total_charges: trades.iter().map(|t| t.charges).sum(),
        max_drawdown,
        trades: trades
            .iter()
            .map(|t| BacktestTradeDto {
                entry_price: t.entry_price,
                exit_price: t.exit_price,
                quantity: t.quantity,
                charges: t.charges,
                pnl: t.pnl,
                entry_t: t.entry_t,
                exit_t: t.exit_t,
            })
            .collect(),
        bars_in_market: 0,
        total_bars: 0,
    })
}

/// The paper record of a strategy, as the dashboard shows it. A running
/// strategy shows the desk's live state; a stopped one its saved state.
fn paper_dto(name: &str, running: bool, record: &PaperRecord) -> PaperStateDto {
    let state = &record.state;
    PaperStateDto {
        name: name.to_owned(),
        running,
        exchange: record.exchange.clone(),
        symbol: record.symbol.clone(),
        interval: record.interval.clone(),
        feed: if record.live_feed { "LIVE" } else { "DELAYED" }.to_owned(),
        approximate: !record.live_feed,
        quantity: state.position.as_ref().map(|p| p.quantity),
        entry_price: state.position.as_ref().map(|p| p.entry_price),
        day_pnl: state.day_pnl,
        realised_pnl: state.realised_pnl,
        charges: state.charges,
        fills: state
            .fills
            .iter()
            .rev()
            .take(DASHBOARD_FILLS)
            .map(|f| PaperFillDto {
                t: f.t,
                side: f.side.into(),
                quantity: f.quantity,
                price: f.price,
                charges: f.charges,
                pnl: f.pnl,
            })
            .collect(),
    }
}

/// Fills per strategy on the dashboard.
const DASHBOARD_FILLS: usize = 20;

fn strategy_named(store: &Store, name: &str) -> Result<nivesh_store::StrategyRow, IpcError> {
    store
        .strategies()?
        .into_iter()
        .find(|r| r.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| IpcError::invalid_argument(format!("no strategy named {name:?}")))
}

/// Starts the paper runner for a strategy on a stream:
/// `PAPER START <NAME> [SYMBOL] [INTERVAL]`, by default the stream it last
/// ran on (RELIANCE daily for a new one). Paper only: nothing here can
/// reach a real broker.
#[tauri::command]
pub async fn paper_start(
    name: String,
    symbol: Option<String>,
    interval: Option<String>,
    services: State<'_, Arc<crate::services::Services>>,
    store: State<'_, Arc<Store>>,
    paper: State<'_, Arc<PaperManager>>,
) -> Result<PaperStateDto, IpcError> {
    let row = strategy_named(&store, &name)?;
    let set: RuleSet = serde_json::from_str(&row.json)
        .map_err(|e| IpcError::internal(format!("stored strategy is unreadable: {e}")))?;
    let last = store
        .paper_state(row.id)?
        .map(|(_, json)| PaperRecord::decode(&json))
        .unwrap_or_default();
    let symbol = symbol.unwrap_or(last.symbol);
    let interval = interval.unwrap_or(last.interval);
    let record = paper
        .start(&services, row.id, set, &last.exchange, &symbol, &interval)
        .await?;
    Ok(paper_dto(&row.name, true, &record))
}

/// Stops (but keeps resumable state for) a paper strategy.
#[tauri::command]
pub async fn paper_stop(
    name: String,
    services: State<'_, Arc<crate::services::Services>>,
    store: State<'_, Arc<Store>>,
    paper: State<'_, Arc<PaperManager>>,
) -> Result<(), IpcError> {
    let row = strategy_named(&store, &name)?;
    paper.stop(&services, row.id)
}

/// Every strategy that has paper state, for the `PAPER` dashboard.
#[tauri::command]
pub async fn paper_states(
    store: State<'_, Arc<Store>>,
    paper: State<'_, Arc<PaperManager>>,
) -> Result<Vec<PaperStateDto>, IpcError> {
    let mut out = Vec::new();
    for row in store.strategies()? {
        let Some((saved_running, json)) = store.paper_state(row.id)? else {
            continue;
        };
        let mut record = PaperRecord::decode(&json);
        let running = paper.is_running(row.id);
        if let Some(live) = paper.state(row.id) {
            record.state = live;
        }
        // Saved as running but not on the desk: resuming failed (logged).
        out.push(paper_dto(&row.name, running || saved_running, &record));
    }
    Ok(out)
}
