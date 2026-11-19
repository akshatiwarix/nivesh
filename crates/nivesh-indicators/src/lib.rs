//! Technical indicators: pure math over `f64` slices.
//!
//! Every indicator has two forms that agree with each other:
//!
//! * a **batch** function that maps a whole series to a series of the same
//!   length, and
//! * a **streaming** struct whose `update` takes one new value and returns the
//!   indicator for it, so the forming bar of a live chart (and, later, a
//!   strategy) never recomputes the whole history.
//!
//! Warm-up: a value that isn't defined yet (the first `n - 1` points of an
//! `n`-period average) is `NaN` in batch output. Streaming single-value
//! indicators return `None` there. Batch and streaming results agree to within
//! floating-point rounding (a property test checks `1e-9` relative).
//!
//! Conventions are the common textbook ones, listed on each function: EMA is
//! seeded with the SMA of the first `n` values, RSI and ATR use Wilder
//! smoothing, Bollinger bands use the population standard deviation.

mod bands;
mod channel;
mod ma;
mod momentum;
mod spec;
mod vwap;

pub use bands::{Atr, Bollinger, BollingerPoint, BollingerSeries, atr, bollinger};
pub use channel::{Highest, Lowest, highest, lowest};
pub use ma::{Ema, Sma, Wma, ema, sma, wma};
pub use momentum::{Macd, MacdPoint, MacdSeries, Rsi, macd, rsi};
pub use spec::{
    IndicatorSpec, Kind, Line, LineStyle, MAX_PERIOD, Ohlcv, Output, Placement, SpecError, compute,
};
pub use vwap::{IST_OFFSET_SECS, Vwap, vwap};

/// The "not yet defined" marker used in batch output.
#[inline]
pub(crate) fn nan() -> f64 {
    f64::NAN
}
