//! Domain types shared by every Nivesh crate.
//!
//! This crate is pure: no I/O, no async, no clocks. Anything that needs the
//! current time takes it as an argument so behaviour is deterministic and
//! testable.

pub mod calendar;
pub mod index;
pub mod instrument;
pub mod market;
pub mod provenance;

pub use calendar::{CalendarError, Date, MarketCalendar, MarketStatus, Session};

pub use index::{CATALOGUE as INDEX_CATALOGUE, IndexId, IndexInfo};
pub use instrument::{Exchange, Instrument, InstrumentId, InstrumentIdError};
pub use market::{
    Candle, CandleSeries, ConstituentsInfo, IndexQuote, IndexSnapshot, Interval, Price, Quote,
    change_from,
};
pub use provenance::{FeedKind, Freshness, Provenance, ProviderId, Sourced};
