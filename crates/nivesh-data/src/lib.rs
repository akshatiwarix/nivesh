//! Market data for Nivesh: where quotes, candles and index levels come from.
//!
//! Providers implement [`DataProvider`]. The rest of the app talks only to the
//! `MarketData` facade (added in a later commit), never to a provider directly.
//! Every value leaves this crate as a `Sourced<T>` carrying its provenance.

pub mod bse;
mod cache;
mod chain;
pub mod clock;
pub mod constituents;
mod error;
pub mod forming;
pub mod health;
pub mod http;
pub mod hub;
pub mod instruments;
pub mod mock;
pub mod nse;
mod provider;
mod singleflight;
mod util;
pub mod yahoo;

pub use cache::{CacheConfig, CachePolicy, PersistentCache, Ttl, daily_fresh_for};
pub use chain::{MarketData, MarketDataBuilder, Routing};
pub use error::DataError;
pub use health::{HealthConfig, HealthState, ProviderStatus};
pub use hub::{HubConfig, HubUpdate, QuoteHub, SubscriptionId, Topic};
pub use instruments::{
    InstrumentIndex, InstrumentMaster, parse_equity_csv, parse_list_csv, write_list_csv,
};
pub use provider::{
    Capabilities, DataProvider, HistoryRange, HistoryRequest, Mover, Movers, MoversRequest,
    NewsItem, NewsRequest,
};
