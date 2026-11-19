//! Wire types for the strategy views: strategies, backtest results and the
//! paper state. Everything here is PAPER.

use nivesh_broker::Side;
use serde::Serialize;
use ts_rs::TS;

/// A saved strategy.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StrategyDto {
    pub id: i64,
    pub name: String,
    pub template: String,
    /// The entry rule's preview, e.g. `SMA(20) crossed above SMA(50)`.
    pub entry: String,
    /// Whether the paper runner is live for it.
    pub running: bool,
    /// Whether paper state exists (stopped-but-resumable).
    pub has_state: bool,
    /// Today's paper P&L when running, rupees.
    pub paper_pnl: Option<f64>,
}

/// One backtest trade.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct BacktestTradeDto {
    pub entry_price: f64,
    pub exit_price: f64,
    pub quantity: i64,
    pub pnl: f64,
    /// UTC ms of the fills.
    pub entry_t: i64,
    pub exit_t: i64,
}

/// A finished backtest.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct BacktestResultDto {
    pub total_pnl: f64,
    pub max_drawdown: f64,
    pub trades: Vec<BacktestTradeDto>,
    pub bars_in_market: usize,
    pub total_bars: usize,
}

/// The paper state of one strategy.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PaperStateDto {
    pub running: bool,
    /// Position symbol, or `null` when flat.
    pub symbol: Option<String>,
    pub quantity: Option<i64>,
    pub entry_price: Option<f64>,
    /// Today's paper P&L, rupees.
    pub day_pnl: f64,
}

/// What `paper_start` reports.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PaperStartDto {
    pub name: String,
    pub started: bool,
    /// The paper state after starting.
    pub state: PaperStateDto,
}

/// Side, for the intents (the gateway's shape).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SideDto {
    Buy,
    Sell,
}

impl From<Side> for SideDto {
    fn from(s: Side) -> Self {
        match s {
            Side::Buy => Self::Buy,
            Side::Sell => Self::Sell,
        }
    }
}
