//! Wire types for charts: columnar candle series and the forming bar.
//!
//! A 5,000-candle series as an array of objects is five times the JSON and a
//! lot of small allocations; as parallel arrays it is what the chart library
//! wants anyway. Timestamps are UTC epoch milliseconds.

use nivesh_core::{Candle, CandleSeries, Sourced};
use serde::Serialize;
use ts_rs::TS;

use crate::ipc::dto::ProvenanceDto;

/// A candle series as parallel columns, oldest first.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HistoryDto {
    pub exchange: String,
    pub symbol: String,
    /// `1m`, `5m`, `1d`, ...
    pub interval: String,
    /// The range that was asked for: `1mo`, `5y`, ...
    pub range: String,
    /// Bar start times (UTC epoch ms).
    pub t: Vec<i64>,
    pub o: Vec<f64>,
    pub h: Vec<f64>,
    pub l: Vec<f64>,
    pub c: Vec<f64>,
    pub v: Vec<f64>,
    pub provenance: ProvenanceDto,
}

impl HistoryDto {
    pub fn new(series: &Sourced<CandleSeries>, range: &str) -> Self {
        let s = &series.data;
        let n = s.candles.len();
        let column = |f: fn(&Candle) -> f64| -> Vec<f64> { s.candles.iter().map(f).collect() };
        let mut t = Vec::with_capacity(n);
        t.extend(s.candles.iter().map(|c| c.t));
        Self {
            exchange: s.id.exchange().code().to_owned(),
            symbol: s.id.symbol().to_owned(),
            interval: s.interval.code().to_owned(),
            range: range.to_owned(),
            t,
            o: column(|c| c.o),
            h: column(|c| c.h),
            l: column(|c| c.l),
            c: column(|c| c.c),
            v: column(|c| c.v),
            provenance: (&series.provenance).into(),
        }
    }
}

/// The bar in progress, pushed over a channel as quotes arrive.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct BarDto {
    pub t: i64,
    pub o: f64,
    pub h: f64,
    pub l: f64,
    pub c: f64,
    pub v: f64,
    /// Where the price that moved this bar came from.
    pub provenance: ProvenanceDto,
}

impl BarDto {
    pub fn new(bar: &Candle, provenance: ProvenanceDto) -> Self {
        Self {
            t: bar.t,
            o: bar.o,
            h: bar.h,
            l: bar.l,
            c: bar.c,
            v: bar.v,
            provenance,
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use nivesh_core::{FeedKind, InstrumentId, Interval, Provenance, ProviderId};

    use super::*;

    #[test]
    fn history_is_columnar_and_keeps_provenance() {
        let now = Utc::now();
        let candle = |t, c| Candle {
            t,
            o: c - 1.0,
            h: c + 1.0,
            l: c - 2.0,
            c,
            v: 10.0,
        };
        let series = Sourced::new(
            CandleSeries {
                id: InstrumentId::parse("NSE:TCS").unwrap(),
                interval: Interval::H4,
                candles: vec![candle(1000, 10.0), candle(2000, 11.0)],
            },
            Provenance::new(ProviderId::Yahoo, FeedKind::Delayed, now, now),
        );
        let dto = HistoryDto::new(&series, "6mo");
        assert_eq!((dto.symbol.as_str(), dto.interval.as_str()), ("TCS", "4h"));
        assert_eq!(dto.t, [1000, 2000]);
        assert_eq!(dto.c, [10.0, 11.0]);
        assert_eq!(dto.provenance.provider, "yahoo");
        let json = serde_json::to_value(&dto).unwrap();
        assert!(json["t"].is_array() && json["asOf"].is_null());
    }
}
