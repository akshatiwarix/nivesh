//! The provider abstraction.

use std::ops::BitOr;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nivesh_core::{
    CandleSeries, Exchange, FeedKind, IndexId, IndexQuote, IndexSnapshot, Instrument, InstrumentId,
    Interval, ProviderId, Quote, Sourced,
};
use serde::{Deserialize, Serialize};

use crate::DataError;

/// What a provider can do. A small bit set, so no extra dependency is needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capabilities(u8);

impl Capabilities {
    pub const NONE: Capabilities = Capabilities(0);
    pub const QUOTE: Capabilities = Capabilities(1);
    pub const HISTORY: Capabilities = Capabilities(1 << 1);
    pub const INDEX: Capabilities = Capabilities(1 << 2);
    pub const MOVERS: Capabilities = Capabilities(1 << 3);
    pub const NEWS: Capabilities = Capabilities(1 << 4);
    pub const INSTRUMENTS: Capabilities = Capabilities(1 << 5);

    pub const fn contains(self, other: Capabilities) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for Capabilities {
    type Output = Capabilities;

    fn bitor(self, rhs: Capabilities) -> Capabilities {
        Capabilities(self.0 | rhs.0)
    }
}

/// How far back a history request reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HistoryRange {
    #[serde(rename = "1d")]
    D1,
    #[serde(rename = "5d")]
    D5,
    #[serde(rename = "1mo")]
    M1,
    #[serde(rename = "3mo")]
    M3,
    #[serde(rename = "6mo")]
    M6,
    #[serde(rename = "1y")]
    Y1,
    #[serde(rename = "5y")]
    Y5,
    #[serde(rename = "max")]
    Max,
}

impl HistoryRange {
    /// Approximate length in days (`Max` is effectively unbounded).
    pub const fn days(self) -> u32 {
        match self {
            HistoryRange::D1 => 1,
            HistoryRange::D5 => 5,
            HistoryRange::M1 => 31,
            HistoryRange::M3 => 92,
            HistoryRange::M6 => 183,
            HistoryRange::Y1 => 366,
            HistoryRange::Y5 => 1826,
            HistoryRange::Max => u32::MAX,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HistoryRequest {
    pub id: InstrumentId,
    pub interval: Interval,
    pub range: HistoryRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MoversRequest {
    pub index: IndexId,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mover {
    pub quote: Quote,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Movers {
    pub gainers: Vec<Mover>,
    pub losers: Vec<Mover>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NewsRequest {
    pub query: Option<String>,
    pub limit: usize,
}

/// Headline, source, time and link only. Never article bodies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewsItem {
    pub title: String,
    pub source: String,
    pub url: String,
    pub published: DateTime<Utc>,
}

/// A source of market data. Every method defaults to
/// [`DataError::Unsupported`], so a provider implements only what it has and
/// advertises it in [`capabilities`](DataProvider::capabilities).
#[async_trait]
pub trait DataProvider: Send + Sync + 'static {
    fn id(&self) -> ProviderId;
    fn capabilities(&self) -> Capabilities;
    fn feed_kind(&self) -> FeedKind;

    /// Quotes for `ids`. Instruments the provider doesn't know are omitted;
    /// if none are known the result is `NotFound`.
    async fn quotes(&self, _ids: &[InstrumentId]) -> Result<Vec<Sourced<Quote>>, DataError> {
        Err(DataError::Unsupported)
    }

    /// Quotes for the members of an index, where a provider can fetch many in
    /// far fewer requests than [`quotes`](Self::quotes) by giving up detail
    /// (Yahoo's batch endpoint has no OHLC or volume). Defaults to `quotes`.
    async fn constituent_quotes(
        &self,
        ids: &[InstrumentId],
    ) -> Result<Vec<Sourced<Quote>>, DataError> {
        self.quotes(ids).await
    }

    async fn history(&self, _req: &HistoryRequest) -> Result<Sourced<CandleSeries>, DataError> {
        Err(DataError::Unsupported)
    }

    /// One index: its level and, where the provider has them, constituents.
    async fn index_snapshot(&self, _index: IndexId) -> Result<Sourced<IndexSnapshot>, DataError> {
        Err(DataError::Unsupported)
    }

    /// Levels of every catalogued index the provider carries, for the MKT
    /// dashboard. Added to the plan's trait so the dashboard costs one request
    /// on NSE instead of one per index. Each level has its own provenance so
    /// the chain can fill indices one provider lacks (SENSEX) from another.
    async fn all_indices(&self) -> Result<Vec<Sourced<IndexQuote>>, DataError> {
        Err(DataError::Unsupported)
    }

    async fn movers(&self, _req: &MoversRequest) -> Result<Sourced<Movers>, DataError> {
        Err(DataError::Unsupported)
    }

    async fn news(&self, _req: &NewsRequest) -> Result<Vec<NewsItem>, DataError> {
        Err(DataError::Unsupported)
    }

    /// The listed equities of `exchange`, for the local instrument master.
    async fn instruments(&self, _exchange: Exchange) -> Result<Vec<Instrument>, DataError> {
        Err(DataError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Bare;

    #[async_trait]
    impl DataProvider for Bare {
        fn id(&self) -> ProviderId {
            ProviderId::Yahoo
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities::NONE
        }
        fn feed_kind(&self) -> FeedKind {
            FeedKind::Delayed
        }
    }

    #[tokio::test]
    async fn every_method_defaults_to_unsupported() {
        let p = Bare;
        let id = InstrumentId::new(Exchange::Nse, "TCS").unwrap();
        assert_eq!(
            p.quotes(std::slice::from_ref(&id)).await.unwrap_err(),
            DataError::Unsupported
        );
        let req = HistoryRequest {
            id,
            interval: Interval::D1,
            range: HistoryRange::M1,
        };
        assert_eq!(p.history(&req).await.unwrap_err(), DataError::Unsupported);
        assert_eq!(
            p.index_snapshot(IndexId::Nifty50).await.unwrap_err(),
            DataError::Unsupported
        );
        assert_eq!(p.all_indices().await.unwrap_err(), DataError::Unsupported);
        let m = MoversRequest {
            index: IndexId::Nifty50,
            limit: 5,
        };
        assert_eq!(p.movers(&m).await.unwrap_err(), DataError::Unsupported);
        let n = NewsRequest {
            query: None,
            limit: 5,
        };
        assert_eq!(p.news(&n).await.unwrap_err(), DataError::Unsupported);
        assert_eq!(
            p.instruments(Exchange::Nse).await.unwrap_err(),
            DataError::Unsupported
        );
    }

    #[test]
    fn capabilities_combine_and_test() {
        let c = Capabilities::QUOTE | Capabilities::INDEX;
        assert!(c.contains(Capabilities::QUOTE));
        assert!(c.contains(Capabilities::INDEX));
        assert!(!c.contains(Capabilities::HISTORY));
        assert!(c.contains(Capabilities::NONE));
    }
}
