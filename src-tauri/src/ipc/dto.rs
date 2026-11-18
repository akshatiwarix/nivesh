//! Wire types for market data. Plain mirrors of the domain types in the shapes
//! the UI wants (camelCase, f64 rupees, epoch milliseconds, string codes), with
//! `From` conversions. No logic beyond field mapping.

use nivesh_core::{
    Exchange, Freshness, IndexId, IndexQuote, IndexSnapshot, Instrument, InstrumentId, Provenance,
    Quote, Sourced,
};
use nivesh_data::{HealthState, HubUpdate, ProviderStatus, Topic};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ipc::IpcError;

/// What the provenance badge shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FreshnessDto {
    Live,
    Delayed,
    Eod,
    Stale,
}

impl From<Freshness> for FreshnessDto {
    fn from(f: Freshness) -> Self {
        match f {
            Freshness::Live => Self::Live,
            Freshness::Delayed => Self::Delayed,
            Freshness::Eod => Self::Eod,
            Freshness::Stale => Self::Stale,
        }
    }
}

/// Where a value came from and how fresh it is. Every market datum carries one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProvenanceDto {
    /// `nse`, `yahoo`, ...
    pub provider: String,
    /// The label to show.
    pub freshness: FreshnessDto,
    /// Timestamp of the data itself (UTC epoch ms).
    pub as_of_ms: i64,
    pub fetched_at_ms: i64,
}

impl From<&Provenance> for ProvenanceDto {
    fn from(p: &Provenance) -> Self {
        Self {
            provider: p.provider.code().to_owned(),
            freshness: p.freshness().into(),
            as_of_ms: p.as_of.timestamp_millis(),
            fetched_at_ms: p.fetched_at.timestamp_millis(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct QuoteDto {
    pub exchange: String,
    pub symbol: String,
    pub name: Option<String>,
    pub ltp: f64,
    pub prev_close: f64,
    pub change: f64,
    pub change_pct: f64,
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub volume: Option<u64>,
    pub week52_high: Option<f64>,
    pub week52_low: Option<f64>,
    pub upper_band: Option<f64>,
    pub lower_band: Option<f64>,
}

impl From<&Quote> for QuoteDto {
    fn from(q: &Quote) -> Self {
        Self {
            exchange: q.id.exchange().code().to_owned(),
            symbol: q.id.symbol().to_owned(),
            name: q.name.clone(),
            ltp: q.ltp,
            prev_close: q.prev_close,
            change: q.change(),
            change_pct: q.change_pct(),
            open: q.open,
            high: q.high,
            low: q.low,
            volume: q.volume,
            week52_high: q.week52_high,
            week52_low: q.week52_low,
            upper_band: q.upper_band,
            lower_band: q.lower_band,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SourcedQuoteDto {
    pub quote: QuoteDto,
    pub provenance: ProvenanceDto,
}

impl From<&Sourced<Quote>> for SourcedQuoteDto {
    fn from(s: &Sourced<Quote>) -> Self {
        Self {
            quote: (&s.data).into(),
            provenance: (&s.provenance).into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct IndexQuoteDto {
    /// Stable code, e.g. `NIFTY50`.
    pub index: String,
    /// Display name, e.g. `NIFTY 50`.
    pub title: String,
    pub last: f64,
    pub prev_close: f64,
    pub change: f64,
    pub change_pct: f64,
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub week52_high: Option<f64>,
    pub week52_low: Option<f64>,
    pub advances: Option<u32>,
    pub declines: Option<u32>,
}

impl From<&IndexQuote> for IndexQuoteDto {
    fn from(q: &IndexQuote) -> Self {
        Self {
            index: q.index.code().to_owned(),
            title: q.index.title().to_owned(),
            last: q.last,
            prev_close: q.prev_close,
            change: q.change(),
            change_pct: q.change_pct(),
            open: q.open,
            high: q.high,
            low: q.low,
            week52_high: q.week52_high,
            week52_low: q.week52_low,
            advances: q.advances,
            declines: q.declines,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SourcedIndexQuoteDto {
    pub quote: IndexQuoteDto,
    pub provenance: ProvenanceDto,
}

impl From<&Sourced<IndexQuote>> for SourcedIndexQuoteDto {
    fn from(s: &Sourced<IndexQuote>) -> Self {
        Self {
            quote: (&s.data).into(),
            provenance: (&s.provenance).into(),
        }
    }
}

/// An index level with its constituents (empty when the source has only the
/// level) under one provenance.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct IndexSnapshotDto {
    pub quote: IndexQuoteDto,
    pub constituents: Vec<QuoteDto>,
    pub provenance: ProvenanceDto,
}

impl From<&Sourced<IndexSnapshot>> for IndexSnapshotDto {
    fn from(s: &Sourced<IndexSnapshot>) -> Self {
        Self {
            quote: (&s.data.quote).into(),
            constituents: s.data.constituents.iter().map(Into::into).collect(),
            provenance: (&s.provenance).into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentDto {
    pub exchange: String,
    pub symbol: String,
    pub name: String,
    pub series: Option<String>,
}

impl From<&Instrument> for InstrumentDto {
    fn from(i: &Instrument) -> Self {
        Self {
            exchange: i.id.exchange().code().to_owned(),
            symbol: i.id.symbol().to_owned(),
            name: i.name.clone(),
            series: i.series.clone(),
        }
    }
}

/// What a view subscribes to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TopicDto {
    Quote { exchange: String, symbol: String },
    Index { index: String },
    AllIndices,
}

impl TopicDto {
    pub fn into_topic(self) -> Result<Topic, IpcError> {
        Ok(match self {
            TopicDto::Quote { exchange, symbol } => {
                let exchange: Exchange =
                    exchange
                        .parse()
                        .map_err(|e: nivesh_core::InstrumentIdError| {
                            IpcError::invalid_argument(e.to_string())
                        })?;
                let id = InstrumentId::new(exchange, &symbol)
                    .map_err(|e| IpcError::invalid_argument(e.to_string()))?;
                Topic::Quote(id)
            }
            TopicDto::Index { index } => Topic::Index(parse_index(&index)?),
            TopicDto::AllIndices => Topic::AllIndices,
        })
    }
}

impl From<&Topic> for TopicDto {
    fn from(t: &Topic) -> Self {
        match t {
            Topic::Quote(id) => Self::Quote {
                exchange: id.exchange().code().to_owned(),
                symbol: id.symbol().to_owned(),
            },
            Topic::Index(index) => Self::Index {
                index: index.code().to_owned(),
            },
            Topic::AllIndices => Self::AllIndices,
        }
    }
}

pub fn parse_index(text: &str) -> Result<IndexId, IpcError> {
    IndexId::find(text).ok_or_else(|| IpcError::invalid_argument(format!("unknown index `{text}`")))
}

/// A message on a quote subscription's channel.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HubUpdateDto {
    Quotes {
        quotes: Vec<SourcedQuoteDto>,
    },
    Index {
        snapshot: IndexSnapshotDto,
    },
    AllIndices {
        rows: Vec<SourcedIndexQuoteDto>,
    },
    /// A topic couldn't be refreshed; the view shows this inline.
    Failed {
        topic: TopicDto,
        error: IpcError,
    },
}

impl From<&HubUpdate> for HubUpdateDto {
    fn from(update: &HubUpdate) -> Self {
        match update {
            HubUpdate::Quotes(q) => Self::Quotes {
                quotes: q.iter().map(Into::into).collect(),
            },
            HubUpdate::Index(s) => Self::Index { snapshot: s.into() },
            HubUpdate::AllIndices(rows) => Self::AllIndices {
                rows: rows.iter().map(Into::into).collect(),
            },
            HubUpdate::Failed { topic, error } => Self::Failed {
                topic: topic.into(),
                error: error.clone().into(),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HealthDto {
    Healthy,
    Degraded,
    Down,
}

impl From<HealthState> for HealthDto {
    fn from(s: HealthState) -> Self {
        match s {
            HealthState::Healthy => Self::Healthy,
            HealthState::Degraded => Self::Degraded,
            HealthState::Down => Self::Down,
        }
    }
}

/// A provider's health, for the status bar (also the `provider:status` event).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatusDto {
    pub provider: String,
    pub state: HealthDto,
    pub consecutive_failures: u32,
    pub last_error: Option<String>,
    pub last_ok_ms: Option<i64>,
}

impl From<&ProviderStatus> for ProviderStatusDto {
    fn from(s: &ProviderStatus) -> Self {
        Self {
            provider: s.provider.code().to_owned(),
            state: s.state.into(),
            consecutive_failures: s.consecutive_failures,
            last_error: s.last_error.clone(),
            last_ok_ms: s.last_ok.map(|t| t.timestamp_millis()),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone as _, Utc};
    use nivesh_core::{FeedKind, ProviderId};
    use nivesh_data::DataError;

    use super::*;
    use crate::ipc::ErrorCode;

    fn id(s: &str) -> InstrumentId {
        InstrumentId::parse(s).unwrap()
    }

    fn quote() -> Quote {
        Quote {
            id: id("RELIANCE"),
            name: Some("Reliance".into()),
            ltp: 110.0,
            prev_close: 100.0,
            open: Some(101.0),
            high: None,
            low: None,
            volume: Some(5),
            week52_high: None,
            week52_low: None,
            upper_band: Some(120.0),
            lower_band: None,
        }
    }

    fn prov(stale: bool) -> Provenance {
        let t = Utc.with_ymd_and_hms(2026, 11, 18, 10, 0, 0).unwrap();
        let p = Provenance::new(ProviderId::Yahoo, FeedKind::Delayed, t, t);
        if stale { p.into_stale() } else { p }
    }

    #[test]
    fn a_quote_serialises_with_derived_change_and_camel_case() {
        let dto = SourcedQuoteDto::from(&Sourced::new(quote(), prov(false)));
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["quote"]["exchange"], "NSE");
        assert_eq!(json["quote"]["symbol"], "RELIANCE");
        assert_eq!(json["quote"]["prevClose"], 100.0);
        assert_eq!(json["quote"]["change"], 10.0);
        assert_eq!(json["quote"]["changePct"], 10.0);
        assert_eq!(json["quote"]["upperBand"], 120.0);
        assert!(json["quote"]["lowerBand"].is_null());
        assert_eq!(json["provenance"]["provider"], "yahoo");
        assert_eq!(json["provenance"]["freshness"], "DELAYED");
        assert_eq!(json["provenance"]["asOfMs"], 1_794_996_000_000_i64);
    }

    #[test]
    fn stale_provenance_is_labelled_stale_never_live() {
        let json = serde_json::to_value(ProvenanceDto::from(&prov(true))).unwrap();
        assert_eq!(json["freshness"], "STALE");
        // Public data can't be labelled LIVE: Yahoo/NSE provenance is DELAYED.
        let json = serde_json::to_value(ProvenanceDto::from(&prov(false))).unwrap();
        assert_ne!(json["freshness"], "LIVE");
    }

    #[test]
    fn topics_parse_from_the_ui_shape_and_reject_bad_input() {
        let t: TopicDto =
            serde_json::from_str(r#"{"kind":"quote","exchange":"nse","symbol":"tcs"}"#).unwrap();
        assert_eq!(t.into_topic().unwrap(), Topic::Quote(id("TCS")));
        let t: TopicDto = serde_json::from_str(r#"{"kind":"index","index":"NIFTY"}"#).unwrap();
        assert_eq!(t.into_topic().unwrap(), Topic::Index(IndexId::Nifty50));
        let t: TopicDto = serde_json::from_str(r#"{"kind":"allIndices"}"#).unwrap();
        assert_eq!(t.into_topic().unwrap(), Topic::AllIndices);

        for bad in [
            r#"{"kind":"quote","exchange":"LSE","symbol":"TCS"}"#,
            r#"{"kind":"quote","exchange":"NSE","symbol":"bad symbol!"}"#,
            r#"{"kind":"index","index":"NOPE"}"#,
        ] {
            let err = serde_json::from_str::<TopicDto>(bad)
                .unwrap()
                .into_topic()
                .unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidArgument, "{bad}");
        }
        assert!(serde_json::from_str::<TopicDto>(r#"{"kind":"weird"}"#).is_err());
    }

    #[test]
    fn a_failed_update_carries_a_structured_error() {
        let update = HubUpdate::Failed {
            topic: Topic::Quote(id("TCS")),
            error: DataError::Timeout,
        };
        let json = serde_json::to_value(HubUpdateDto::from(&update)).unwrap();
        assert_eq!(json["kind"], "failed");
        assert_eq!(json["topic"]["kind"], "quote");
        assert_eq!(json["error"]["code"], "UNAVAILABLE");
        assert_eq!(json["error"]["retryable"], true);
    }

    #[test]
    fn index_snapshot_without_constituents_has_an_empty_list() {
        let level = IndexQuote {
            index: IndexId::Nifty50,
            last: 25000.0,
            prev_close: 24900.0,
            open: None,
            high: None,
            low: None,
            week52_high: None,
            week52_low: None,
            advances: Some(11),
            declines: Some(39),
        };
        let snap = Sourced::new(
            IndexSnapshot {
                quote: level,
                constituents: vec![],
            },
            prov(false),
        );
        let json = serde_json::to_value(IndexSnapshotDto::from(&snap)).unwrap();
        assert_eq!(json["quote"]["index"], "NIFTY50");
        assert_eq!(json["quote"]["title"], "NIFTY 50");
        assert_eq!(json["quote"]["advances"], 11);
        assert_eq!(json["constituents"], serde_json::json!([]));
    }

    #[test]
    fn provider_status_maps_health_states() {
        let status = ProviderStatus {
            provider: ProviderId::Nse,
            state: HealthState::Down,
            consecutive_failures: 3,
            last_error: Some("blocked by the provider".into()),
            last_ok: None,
        };
        let json = serde_json::to_value(ProviderStatusDto::from(&status)).unwrap();
        assert_eq!(json["provider"], "nse");
        assert_eq!(json["state"], "DOWN");
        assert_eq!(json["consecutiveFailures"], 3);
        assert!(json["lastOkMs"].is_null());
    }
}
