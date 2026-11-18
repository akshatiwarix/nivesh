//! Market data for Nivesh: where quotes, candles and index levels come from.
//!
//! Providers implement [`DataProvider`]. The rest of the app talks only to the
//! `MarketData` facade (added in a later commit), never to a provider directly.
//! Every value leaves this crate as a `Sourced<T>` carrying its provenance.

mod error;
pub mod http;
pub mod mock;
pub mod nse;
mod provider;
mod util;
pub mod yahoo;

pub use error::DataError;
pub use provider::{
    Capabilities, DataProvider, HistoryRange, HistoryRequest, Mover, Movers, MoversRequest,
    NewsItem, NewsRequest,
};
