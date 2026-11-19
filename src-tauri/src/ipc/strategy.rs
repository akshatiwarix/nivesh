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

/// One paper fill.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PaperFillDto {
    /// UTC ms of the bar whose open filled it.
    pub t: i64,
    pub side: SideDto,
    pub quantity: i64,
    /// Rupees.
    pub price: f64,
    /// Realised P&L of the round trip, on the closing fill.
    pub pnl: Option<f64>,
}

/// The paper state of one strategy. Everything here is PAPER.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PaperStateDto {
    pub name: String,
    pub running: bool,
    /// The stream it trades: exchange, symbol and bar interval.
    pub exchange: String,
    pub symbol: String,
    pub interval: String,
    /// `LIVE` when the bars come from the broker's real-time feed,
    /// otherwise `DELAYED`.
    pub feed: String,
    /// True on DELAYED bars: the paper fills are approximate.
    pub approximate: bool,
    /// The open position, or `null`s when flat.
    pub quantity: Option<i64>,
    pub entry_price: Option<f64>,
    /// Today's (IST) realised paper P&L, rupees.
    pub day_pnl: f64,
    /// Realised paper P&L since the strategy first started, rupees.
    pub realised_pnl: f64,
    /// The newest fills, newest first.
    pub fills: Vec<PaperFillDto>,
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

impl From<nivesh_strategy::IntentSide> for SideDto {
    fn from(s: nivesh_strategy::IntentSide) -> Self {
        match s {
            nivesh_strategy::IntentSide::Buy => Self::Buy,
            nivesh_strategy::IntentSide::Sell => Self::Sell,
        }
    }
}

impl From<Side> for SideDto {
    fn from(s: Side) -> Self {
        match s {
            Side::Buy => Self::Buy,
            Side::Sell => Self::Sell,
        }
    }
}
