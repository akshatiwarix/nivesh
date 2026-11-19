//! The provider abstraction.

use std::ops::BitOr;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nivesh_core::{
    Candle, CandleSeries, Exchange, FeedKind, IndexId, IndexQuote, IndexSnapshot, Instrument,
    InstrumentId, Interval, Provenance, ProviderId, Quote, Sourced,
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

impl HistoryRange {
    pub const ALL: [HistoryRange; 8] = [
        HistoryRange::D1,
        HistoryRange::D5,
        HistoryRange::M1,
        HistoryRange::M3,
        HistoryRange::M6,
        HistoryRange::Y1,
        HistoryRange::Y5,
        HistoryRange::Max,
    ];

    /// The short code, as the provider and the command line spell it.
    pub const fn code(self) -> &'static str {
        match self {
            HistoryRange::D1 => "1d",
            HistoryRange::D5 => "5d",
            HistoryRange::M1 => "1mo",
            HistoryRange::M3 => "3mo",
            HistoryRange::M6 => "6mo",
            HistoryRange::Y1 => "1y",
            HistoryRange::Y5 => "5y",
            HistoryRange::Max => "max",
        }
    }

    /// How much history a chart opens with: as much as the public sources
    /// serve for that interval, without asking for something they refuse.
    pub const fn default_for(interval: Interval) -> HistoryRange {
        match interval {
            Interval::M1 => HistoryRange::D5,
            Interval::M5 | Interval::M15 | Interval::M30 => HistoryRange::M1,
            Interval::H1 | Interval::H4 => HistoryRange::M6,
            Interval::D1 => HistoryRange::Y5,
            Interval::W1 => HistoryRange::Max,
            Interval::Mo1 => HistoryRange::Max,
        }
    }
}

impl std::str::FromStr for HistoryRange {
    type Err = String;

    /// `1D`, `5D`, `1MO`, `3MO`, `6MO`, `1Y`, `5Y`, `MAX`, case-insensitively.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim().to_ascii_lowercase();
        HistoryRange::ALL
            .into_iter()
            .find(|r| r.code() == s)
            .ok_or_else(|| {
                format!("unknown range `{s}` (use 1D, 5D, 1MO, 3MO, 6MO, 1Y, 5Y or MAX)")
            })
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

/// Top movers of one index universe. The gainers and losers come from one
/// source; the most-active list is from that source too when it reports
/// volumes, and otherwise it is an EOD ranking with its own provenance
/// (see [`Movers::most_active_info`]).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Movers {
    pub gainers: Vec<Mover>,
    pub losers: Vec<Mover>,
    /// By traded volume. Empty when the source gives no volume per stock.
    #[serde(default)]
    pub most_active: Vec<Mover>,
    /// Set only when most-active is ranked from the last full session's
    /// traded value (the daily bhavcopy) instead of this source's own
    /// volumes. The rows then come from whichever provider priced the
    /// index's constituents, so they carry their own provenance.
    #[serde(default)]
    pub most_active_info: Option<MostActiveInfo>,
}

/// Provenance of an EOD-ranked most-active list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MostActiveInfo {
    /// Provenance of the constituent quotes the rows come from.
    pub provenance: Provenance,
    /// `YYYY-MM-DD` of the bhavcopy session the ranking came from.
    pub ranked_as_of: String,
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
    /// NSE symbols of the listed companies the headline mentions.
    #[serde(default)]
    pub tags: Vec<String>,
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

    /// Daily candles of an index, for sector returns and the like. Candles
    /// only: an index has no instrument id.
    async fn index_history(
        &self,
        _index: IndexId,
        _range: HistoryRange,
    ) -> Result<Sourced<Vec<Candle>>, DataError> {
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
    fn ranges_parse_and_have_defaults_every_interval_can_serve() {
        assert_eq!("1mo".parse(), Ok(HistoryRange::M1));
        assert_eq!("MAX".parse(), Ok(HistoryRange::Max));
        assert!("2y".parse::<HistoryRange>().is_err());
        for interval in Interval::ALL {
            let range = HistoryRange::default_for(interval);
            assert!(
                crate::yahoo::check_limits(interval, range).is_ok(),
                "{interval} default {range:?} must be servable"
            );
        }
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
