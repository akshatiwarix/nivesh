//! Wire types for the market views: movers, breadth, sectors and news.

use nivesh_core::Sourced;
use nivesh_data::{DmaBreadth, IndexBreadth, Movers, NewsItem, SectorPerformance};
use serde::Serialize;
use ts_rs::TS;

use crate::ipc::dto::{ProvenanceDto, QuoteDto};

/// Top movers of one index universe, from one source.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MoversDto {
    /// Index code, e.g. `NIFTY50`.
    pub index: String,
    pub gainers: Vec<QuoteDto>,
    pub losers: Vec<QuoteDto>,
    /// By volume. Empty when the source reports no volume per stock.
    pub most_active: Vec<QuoteDto>,
    pub provenance: ProvenanceDto,
}

impl MoversDto {
    pub fn new(index: &str, movers: &Sourced<Movers>) -> Self {
        let rows =
            |list: &[nivesh_data::Mover]| list.iter().map(|m| QuoteDto::from(&m.quote)).collect();
        Self {
            index: index.to_owned(),
            gainers: rows(&movers.data.gainers),
            losers: rows(&movers.data.losers),
            most_active: rows(&movers.data.most_active),
            provenance: (&movers.provenance).into(),
        }
    }
}

/// Advances, declines and unchanged for one index.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct BreadthDto {
    pub index: String,
    pub advances: u32,
    pub declines: u32,
    pub unchanged: u32,
    pub provenance: ProvenanceDto,
}

impl From<&Sourced<IndexBreadth>> for BreadthDto {
    fn from(b: &Sourced<IndexBreadth>) -> Self {
        Self {
            index: b.data.index.code().to_owned(),
            advances: b.data.breadth.advances,
            declines: b.data.breadth.declines,
            unchanged: b.data.breadth.unchanged,
            provenance: (&b.provenance).into(),
        }
    }
}

/// One moving-average period of the sweep.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DmaPeriodDto {
    /// 20, 50 or 200 (trading days).
    pub period: u32,
    /// Stocks with enough history to measure.
    pub measured: u32,
    pub above: u32,
    /// `above / measured * 100`; `null` when nothing could be measured.
    pub percent: Option<f64>,
}

/// Share of NIFTY 50 or NIFTY 100 above their moving averages.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DmaBreadthDto {
    pub universe: String,
    pub members: u32,
    pub periods: Vec<DmaPeriodDto>,
    /// Date and publisher of the membership list used.
    pub members_as_of: String,
    pub members_source: String,
    pub provenance: ProvenanceDto,
}

impl From<&Sourced<DmaBreadth>> for DmaBreadthDto {
    fn from(d: &Sourced<DmaBreadth>) -> Self {
        Self {
            universe: d.data.universe.code().to_owned(),
            members: d.data.members,
            periods: d
                .data
                .periods
                .iter()
                .map(|p| DmaPeriodDto {
                    period: p.period,
                    measured: p.measured,
                    above: p.above,
                    percent: p.percent(),
                })
                .collect(),
            members_as_of: d.data.members_info.as_of.clone(),
            members_source: d.data.members_info.source.clone(),
            provenance: (&d.provenance).into(),
        }
    }
}

/// Progress of the moving-average sweep, over a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProgressDto {
    pub done: u32,
    pub total: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SectorRowDto {
    /// Index code, e.g. `NIFTYIT`.
    pub index: String,
    pub title: String,
    pub last: f64,
    /// Percent returns; `null` where the history doesn't reach back far enough.
    pub d1: Option<f64>,
    pub w1: Option<f64>,
    pub m1: Option<f64>,
    pub ytd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SectorsDto {
    pub rows: Vec<SectorRowDto>,
    pub provenance: ProvenanceDto,
}

impl From<&Sourced<SectorPerformance>> for SectorsDto {
    fn from(s: &Sourced<SectorPerformance>) -> Self {
        Self {
            rows: s
                .data
                .rows
                .iter()
                .map(|r| SectorRowDto {
                    index: r.index.code().to_owned(),
                    title: r.index.title().to_owned(),
                    last: r.last,
                    d1: r.returns.d1,
                    w1: r.returns.w1,
                    m1: r.returns.m1,
                    ytd: r.returns.ytd,
                })
                .collect(),
            provenance: (&s.provenance).into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use nivesh_core::{ConstituentsInfo, FeedKind, IndexId, Provenance, ProviderId};
    use nivesh_data::analytics::{AboveMa, Returns};
    use nivesh_data::{DmaUniverse, SectorRow};

    use super::*;

    fn prov() -> Provenance {
        let now = Utc::now();
        Provenance::new(ProviderId::Yahoo, FeedKind::Delayed, now, now)
    }

    #[test]
    fn dma_percent_is_null_when_nothing_was_measured() {
        let d = Sourced::new(
            DmaBreadth {
                universe: DmaUniverse::Nifty50,
                members: 50,
                periods: vec![
                    AboveMa {
                        period: 20,
                        measured: 40,
                        above: 10,
                    },
                    AboveMa {
                        period: 200,
                        measured: 0,
                        above: 0,
                    },
                ],
                members_info: ConstituentsInfo {
                    as_of: "2026-11-18".into(),
                    source: "NSE Indices".into(),
                },
            },
            prov(),
        );
        let dto = DmaBreadthDto::from(&d);
        assert_eq!(dto.universe, "NIFTY50");
        assert_eq!(dto.periods[0].percent, Some(25.0));
        assert_eq!(dto.periods[1].percent, None);
        assert_eq!(dto.members_as_of, "2026-11-18");
    }

    #[test]
    fn sector_rows_carry_titles_and_missing_returns_as_null() {
        let s = Sourced::new(
            SectorPerformance {
                rows: vec![SectorRow {
                    index: IndexId::NiftyIt,
                    last: 40_000.0,
                    returns: Returns {
                        d1: Some(1.5),
                        w1: None,
                        m1: Some(-2.0),
                        ytd: None,
                    },
                }],
            },
            prov(),
        );
        let dto = SectorsDto::from(&s);
        assert_eq!(
            (dto.rows[0].index.as_str(), dto.rows[0].title.as_str()),
            ("NIFTYIT", "NIFTY IT")
        );
        let json = serde_json::to_value(&dto).unwrap();
        assert!(json["rows"][0]["w1"].is_null() && json["rows"][0]["d1"] == 1.5);
    }
}

/// A headline. Text and a link only: no bodies, no HTML.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NewsItemDto {
    pub title: String,
    pub source: String,
    /// Always http or https (checked when the feed is parsed).
    pub url: String,
    pub published_ms: i64,
    /// NSE symbols of companies the headline mentions.
    pub tags: Vec<String>,
}

impl From<&NewsItem> for NewsItemDto {
    fn from(n: &NewsItem) -> Self {
        Self {
            title: n.title.clone(),
            source: n.source.clone(),
            url: n.url.clone(),
            published_ms: n.published.timestamp_millis(),
            tags: n.tags.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NewsDto {
    /// The symbol filter that was applied, if any.
    pub query: Option<String>,
    pub items: Vec<NewsItemDto>,
    pub provenance: ProvenanceDto,
}

impl NewsDto {
    pub fn new(query: Option<String>, news: &Sourced<Vec<NewsItem>>) -> Self {
        Self {
            query,
            items: news.data.iter().map(Into::into).collect(),
            provenance: (&news.provenance).into(),
        }
    }
}
