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
    /// Charges on both fills, rupees.
    pub charges: f64,
    /// Net of charges, rupees.
    pub pnl: f64,
    /// UTC ms of the fills.
    pub entry_t: i64,
    pub exit_t: i64,
}

/// One walk-forward window.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WindowDto {
    /// UTC ms of the window's first and last bar.
    pub start_t: i64,
    pub end_t: i64,
    pub trades: usize,
    /// Winning trades over all trades; `null` without trades.
    pub win_rate: Option<f64>,
    /// Net of charges, rupees.
    pub net_pnl: f64,
    pub charges: f64,
    pub max_drawdown: f64,
}

impl From<&nivesh_strategy::WindowResult> for WindowDto {
    fn from(w: &nivesh_strategy::WindowResult) -> Self {
        Self {
            start_t: w.start_t,
            end_t: w.end_t,
            trades: w.trades,
            win_rate: w.win_rate(),
            net_pnl: w.net_pnl,
            charges: w.charges,
            max_drawdown: w.max_drawdown,
        }
    }
}

/// The out-of-sample and walk-forward check of a backtest.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WalkForwardDto {
    /// The first 70% of the bars, and the last 30%.
    pub in_sample: WindowDto,
    pub out_of_sample: WindowDto,
    /// Consecutive equal windows, oldest first.
    pub windows: Vec<WindowDto>,
    pub profitable_windows: usize,
    pub worst_window_pnl: f64,
}

impl From<&nivesh_strategy::WalkForward> for WalkForwardDto {
    fn from(w: &nivesh_strategy::WalkForward) -> Self {
        Self {
            in_sample: (&w.in_sample).into(),
            out_of_sample: (&w.out_of_sample).into(),
            windows: w.windows.iter().map(WindowDto::from).collect(),
            profitable_windows: w.profitable_windows,
            worst_window_pnl: w.worst_window_pnl,
        }
    }
}

/// A finished backtest.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct BacktestResultDto {
    /// The instrument it ran on, `NSE:RELIANCE` shaped.
    pub symbol: String,
    /// The bar interval and history range, e.g. `1D` and `1Y`.
    pub interval: String,
    pub range: String,
    /// Net of charges and slippage, rupees.
    pub total_pnl: f64,
    /// Charges paid over the run, rupees.
    pub total_charges: f64,
    pub max_drawdown: f64,
    pub trades: Vec<BacktestTradeDto>,
    pub bars_in_market: usize,
    pub total_bars: usize,
    /// The out-of-sample / walk-forward check; `null` when the series was
    /// too short to split, or for runs saved before it existed.
    pub walk_forward: Option<WalkForwardDto>,
}

/// One paper fill.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PaperFillDto {
    /// UTC ms of the bar whose open filled it.
    pub t: i64,
    pub side: SideDto,
    pub quantity: i64,
    /// Rupees, after slippage.
    pub price: f64,
    /// Charges on this fill, rupees.
    pub charges: f64,
    /// Realised P&L of the round trip net of charges, on the closing fill.
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
    /// Realised paper P&L since the strategy first started, net of
    /// charges, rupees.
    pub realised_pnl: f64,
    /// Charges paid since the strategy first started, rupees.
    pub charges: f64,
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
