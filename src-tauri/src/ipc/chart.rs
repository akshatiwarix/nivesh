//! Wire types for charts: columnar candle series and the forming bar.
//!
//! A 5,000-candle series as an array of objects is five times the JSON and a
//! lot of small allocations; as parallel arrays it is what the chart library
//! wants anyway. Timestamps are UTC epoch milliseconds.

use nivesh_core::{Candle, CandleSeries, Sourced};
use nivesh_indicators::{LineStyle, Output, Placement};
use serde::{Deserialize, Serialize};
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
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

    pub fn candle(&self) -> Candle {
        Candle {
            t: self.t,
            o: self.o,
            h: self.h,
            l: self.l,
            c: self.c,
            v: self.v,
        }
    }
}

/// The last indicator values with the forming bar folded in, aligned to
/// `compute_indicators`' indicators and lines, so the chart can move an
/// indicator's newest point without recomputing the whole series.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct IndicatorPreviewDto {
    /// The bar time the values were computed at, epoch ms.
    pub t: i64,
    /// One entry per requested spec, in the order given.
    pub indicators: Vec<IndicatorPreviewLinesDto>,
}

/// One indicator's last values. `lines` matches `IndicatorDto::lines` order
/// for the same id; `null` while the indicator is still warming up.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct IndicatorPreviewLinesDto {
    /// Canonical spec id, e.g. `SMA 50`.
    pub id: String,
    pub lines: Vec<Option<f64>>,
}

/// One indicator's drawable lines, aligned to `IndicatorsDto::t`. A `null`
/// value means the indicator is not defined there yet (warm-up).
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct IndicatorDto {
    /// Canonical spec text: `SMA 50`, `MACD 12 26 9`.
    pub id: String,
    /// `SMA`, `RSI`, ... (what `RM` takes).
    pub kind: String,
    /// `overlay` on the price pane, or `pane` for its own.
    pub placement: String,
    pub lines: Vec<IndicatorLineDto>,
    /// Fixed value range of its pane, e.g. RSI 0 to 100.
    pub bounds: Option<(f64, f64)>,
    /// Reference levels to draw, e.g. RSI 30 and 70.
    pub guides: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct IndicatorLineDto {
    pub name: String,
    /// `line` or `histogram`.
    pub style: String,
    pub values: Vec<Option<f64>>,
}

/// Indicators for one candle series. `t` is the series' bar times, so the view
/// can align the lines to the candles it drew even if a refresh moved them.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct IndicatorsDto {
    pub t: Vec<i64>,
    pub indicators: Vec<IndicatorDto>,
}

impl From<&Output> for IndicatorDto {
    fn from(o: &Output) -> Self {
        Self {
            id: o.id.clone(),
            kind: o.kind.code().to_owned(),
            placement: match o.placement {
                Placement::Overlay => "overlay",
                Placement::Pane => "pane",
            }
            .to_owned(),
            lines: o
                .lines
                .iter()
                .map(|l| IndicatorLineDto {
                    name: l.name.clone(),
                    style: match l.style {
                        LineStyle::Line => "line",
                        LineStyle::Histogram => "histogram",
                    }
                    .to_owned(),
                    values: l
                        .values
                        .iter()
                        .map(|v| v.is_finite().then_some(*v))
                        .collect(),
                })
                .collect(),
            bounds: o.bounds,
            guides: o.guides.clone(),
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

    #[test]
    fn indicator_gaps_become_null_not_nan() {
        use nivesh_indicators::{IndicatorSpec, Ohlcv, compute};
        let t = [0, 1, 2, 3];
        let c = [1.0, 2.0, 3.0, 4.0];
        let spec: IndicatorSpec = "SMA 3".parse().unwrap();
        let out = compute(
            &spec,
            &Ohlcv {
                t: &t,
                h: &c,
                l: &c,
                c: &c,
                v: &c,
            },
        );
        let dto = IndicatorDto::from(&out);
        assert_eq!(dto.placement, "overlay");
        assert_eq!(dto.lines[0].values, [None, None, Some(2.0), Some(3.0)]);
        let json = serde_json::to_string(&dto).unwrap();
        assert!(json.contains("null") && !json.contains("NaN"));
    }
}
