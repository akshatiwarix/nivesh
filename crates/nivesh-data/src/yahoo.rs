//! Yahoo Finance provider, via the public chart endpoint
//! (`/v8/finance/chart/<ticker>`).
//!
//! The chart endpoint needs no cookie or crumb, unlike the quote endpoint. It
//! serves quotes (from `meta`), index levels and candle history. Yahoo marks
//! Indian data as delayed, so everything here is [`FeedKind::Delayed`].
//!
//! Interval and range limits are checked before any request and reported as
//! [`DataError::UnsupportedRange`], so the UI can explain them.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use nivesh_core::{
    Candle, CandleSeries, Exchange, FeedKind, IndexId, IndexQuote, IndexSnapshot, InstrumentId,
    Interval, Provenance, ProviderId, Quote, Sourced,
};
use serde::Deserialize;
use tokio::task::JoinSet;

use crate::http::HttpClient;
use crate::util::{percent_encode, settle};
use crate::{Capabilities, DataError, DataProvider, HistoryRange, HistoryRequest};

pub const DEFAULT_BASE_URL: &str = "https://query1.finance.yahoo.com";

/// Yahoo tickers for catalogued indices. A data table, not logic: some sector
/// tickers are missing or unreliable on Yahoo, and the live smoke test reports
/// which ones answer. Indices absent from this table are skipped.
pub const INDEX_TICKERS: &[(IndexId, &str)] = &[
    (IndexId::Nifty50, "^NSEI"),
    (IndexId::Sensex, "^BSESN"),
    (IndexId::NiftyBank, "^NSEBANK"),
    (IndexId::NiftyIt, "^CNXIT"),
    (IndexId::NiftyAuto, "^CNXAUTO"),
    (IndexId::NiftyPharma, "^CNXPHARMA"),
    (IndexId::NiftyFmcg, "^CNXFMCG"),
    (IndexId::NiftyMetal, "^CNXMETAL"),
    (IndexId::NiftyRealty, "^CNXREALTY"),
    (IndexId::NiftyEnergy, "^CNXENERGY"),
    (IndexId::NiftyPsuBank, "^CNXPSUBANK"),
    (IndexId::NiftyFinService, "NIFTY_FIN_SERVICE.NS"),
    (IndexId::NiftyMedia, "^CNXMEDIA"),
];

pub fn index_ticker(index: IndexId) -> Option<&'static str> {
    INDEX_TICKERS
        .iter()
        .find(|(i, _)| *i == index)
        .map(|(_, t)| *t)
}

/// `RELIANCE.NS`, `500325.BO`.
pub fn equity_ticker(id: &InstrumentId) -> String {
    let suffix = match id.exchange() {
        Exchange::Nse => "NS",
        Exchange::Bse => "BO",
    };
    format!("{}.{suffix}", id.symbol())
}

const fn yahoo_interval(interval: Interval) -> &'static str {
    match interval {
        Interval::M1 => "1m",
        Interval::M5 => "5m",
        Interval::M15 => "15m",
        Interval::M30 => "30m",
        Interval::H1 => "60m",
        Interval::D1 => "1d",
        Interval::W1 => "1wk",
    }
}

const fn yahoo_range(range: HistoryRange) -> &'static str {
    match range {
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

/// How far back Yahoo serves each interval, in days (approximate, from Yahoo's
/// documented behaviour: 1m ≈ 7 days, 2–30m ≈ 60 days, 60m ≈ 730 days).
const fn max_days(interval: Interval) -> Option<(u32, &'static str)> {
    match interval {
        Interval::M1 => Some((7, "1-minute bars cover only the last 7 days")),
        Interval::M5 | Interval::M15 | Interval::M30 => {
            Some((60, "intraday bars under 1 hour cover only the last 60 days"))
        }
        Interval::H1 => Some((730, "hourly bars cover only the last 2 years")),
        Interval::D1 | Interval::W1 => None,
    }
}

/// Rejects interval/range combinations Yahoo can't serve.
pub fn check_limits(interval: Interval, range: HistoryRange) -> Result<(), DataError> {
    match max_days(interval) {
        Some((limit, why)) if range.days() > limit => Err(DataError::UnsupportedRange(format!(
            "Yahoo: {why}; ask for a shorter range or a wider interval"
        ))),
        _ => Ok(()),
    }
}

// ---- response model -------------------------------------------------------

#[derive(Deserialize)]
struct ChartEnvelope {
    chart: Chart,
}

#[derive(Deserialize)]
struct Chart {
    result: Option<Vec<ChartResult>>,
    error: Option<ChartError>,
}

#[derive(Deserialize)]
struct ChartError {
    code: Option<String>,
    description: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChartResult {
    meta: Meta,
    #[serde(default)]
    timestamp: Vec<i64>,
    indicators: Option<Indicators>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Meta {
    regular_market_price: Option<f64>,
    regular_market_time: Option<i64>,
    previous_close: Option<f64>,
    chart_previous_close: Option<f64>,
    regular_market_day_high: Option<f64>,
    regular_market_day_low: Option<f64>,
    regular_market_volume: Option<u64>,
    fifty_two_week_high: Option<f64>,
    fifty_two_week_low: Option<f64>,
    long_name: Option<String>,
    short_name: Option<String>,
}

#[derive(Deserialize)]
struct Indicators {
    #[serde(default)]
    quote: Vec<Bars>,
}

#[derive(Deserialize, Default)]
struct Bars {
    #[serde(default)]
    open: Vec<Option<f64>>,
    #[serde(default)]
    high: Vec<Option<f64>>,
    #[serde(default)]
    low: Vec<Option<f64>>,
    #[serde(default)]
    close: Vec<Option<f64>>,
    #[serde(default)]
    volume: Vec<Option<f64>>,
}

fn parse_chart(body: &[u8]) -> Result<ChartResult, DataError> {
    let env: ChartEnvelope =
        serde_json::from_slice(body).map_err(|e| DataError::Parse(format!("yahoo chart: {e}")))?;
    if let Some(err) = env.chart.error {
        let description = err.description.unwrap_or_default();
        let code = err.code.unwrap_or_default();
        return Err(match code.as_str() {
            "Not Found" => DataError::NotFound,
            _ if description.contains("range") || description.contains("not available") => {
                DataError::UnsupportedRange(format!("Yahoo: {description}"))
            }
            _ => DataError::Upstream(format!("yahoo: {code}: {description}")),
        });
    }
    env.chart
        .result
        .and_then(|mut r| (!r.is_empty()).then(|| r.swap_remove(0)))
        .ok_or_else(|| DataError::Parse("yahoo chart: empty result".into()))
}

fn utc_from_secs(secs: i64) -> Result<DateTime<Utc>, DataError> {
    DateTime::from_timestamp(secs, 0)
        .ok_or_else(|| DataError::Parse(format!("bad timestamp {secs}")))
}

fn last_value(values: &[Option<f64>]) -> Option<f64> {
    values.iter().rev().find_map(|v| *v)
}

fn provenance(as_of: DateTime<Utc>) -> Provenance {
    Provenance::new(ProviderId::Yahoo, FeedKind::Delayed, as_of, Utc::now())
}

/// The fields common to stock and index quotes.
struct Level {
    last: f64,
    prev_close: f64,
    open: Option<f64>,
    high: Option<f64>,
    low: Option<f64>,
    as_of: DateTime<Utc>,
}

fn level(result: &ChartResult) -> Result<Level, DataError> {
    let meta = &result.meta;
    let bars = result.indicators.as_ref().and_then(|i| i.quote.first());
    let last = meta
        .regular_market_price
        .ok_or_else(|| DataError::Parse("yahoo chart: no regularMarketPrice".into()))?;
    let prev_close = meta
        .previous_close
        .or(meta.chart_previous_close)
        .ok_or_else(|| DataError::Parse("yahoo chart: no previous close".into()))?;
    let as_of = utc_from_secs(
        meta.regular_market_time
            .or_else(|| result.timestamp.last().copied())
            .ok_or_else(|| DataError::Parse("yahoo chart: no timestamp".into()))?,
    )?;
    Ok(Level {
        last,
        prev_close,
        open: bars.and_then(|b| last_value(&b.open)),
        high: meta
            .regular_market_day_high
            .or_else(|| bars.and_then(|b| last_value(&b.high))),
        low: meta
            .regular_market_day_low
            .or_else(|| bars.and_then(|b| last_value(&b.low))),
        as_of,
    })
}

pub fn parse_quote(body: &[u8], id: &InstrumentId) -> Result<Sourced<Quote>, DataError> {
    let result = parse_chart(body)?;
    let l = level(&result)?;
    let meta = &result.meta;
    let quote = Quote {
        id: id.clone(),
        name: meta.long_name.clone().or_else(|| meta.short_name.clone()),
        ltp: l.last,
        prev_close: l.prev_close,
        open: l.open,
        high: l.high,
        low: l.low,
        // Yahoo reports 0 volume for some instruments; treat that as unknown.
        volume: meta.regular_market_volume.filter(|v| *v > 0),
        week52_high: meta.fifty_two_week_high,
        week52_low: meta.fifty_two_week_low,
        upper_band: None,
        lower_band: None,
    };
    Ok(Sourced::new(quote, provenance(l.as_of)))
}

pub fn parse_index_quote(body: &[u8], index: IndexId) -> Result<Sourced<IndexQuote>, DataError> {
    let result = parse_chart(body)?;
    let l = level(&result)?;
    let quote = IndexQuote {
        index,
        last: l.last,
        prev_close: l.prev_close,
        open: l.open,
        high: l.high,
        low: l.low,
        week52_high: result.meta.fifty_two_week_high,
        week52_low: result.meta.fifty_two_week_low,
        advances: None,
        declines: None,
    };
    Ok(Sourced::new(quote, provenance(l.as_of)))
}

pub fn parse_history(
    body: &[u8],
    req: &HistoryRequest,
) -> Result<Sourced<CandleSeries>, DataError> {
    let result = parse_chart(body)?;
    let bars = result
        .indicators
        .as_ref()
        .and_then(|i| i.quote.first())
        .ok_or_else(|| DataError::Parse("yahoo chart: no quote indicators".into()))?;
    let n = result.timestamp.len();
    if [
        bars.open.len(),
        bars.high.len(),
        bars.low.len(),
        bars.close.len(),
    ]
    .iter()
    .any(|len| *len != n)
    {
        return Err(DataError::Parse(
            "yahoo chart: column lengths differ".into(),
        ));
    }
    let mut candles = Vec::with_capacity(n);
    for i in 0..n {
        // Yahoo leaves null rows for halted or missing bars; skip them.
        let (Some(o), Some(h), Some(l), Some(c)) =
            (bars.open[i], bars.high[i], bars.low[i], bars.close[i])
        else {
            continue;
        };
        let v = bars.volume.get(i).copied().flatten().unwrap_or(0.0);
        candles.push(Candle {
            t: result.timestamp[i].saturating_mul(1000),
            o,
            h,
            l,
            c,
            v,
        });
    }
    let series = CandleSeries {
        id: req.id.clone(),
        interval: req.interval,
        candles,
    };
    if !series.is_ordered() {
        return Err(DataError::Parse(
            "yahoo chart: timestamps are not increasing".into(),
        ));
    }
    let as_of = match series.candles.last() {
        Some(c) => utc_from_secs(c.t / 1000)?,
        None => return Err(DataError::NotFound),
    };
    Ok(Sourced::new(series, provenance(as_of)))
}

// ---- provider -------------------------------------------------------------

pub struct YahooProvider {
    http: HttpClient,
    base_url: String,
}

impl YahooProvider {
    pub fn new(http: HttpClient) -> Self {
        Self::with_base_url(http, DEFAULT_BASE_URL)
    }

    /// For tests and local fixture servers.
    pub fn with_base_url(http: HttpClient, base_url: &str) -> Self {
        Self {
            http,
            base_url: base_url.trim_end_matches('/').to_owned(),
        }
    }

    fn chart_url(&self, ticker: &str, range: &str, interval: &str) -> String {
        format!(
            "{}/v8/finance/chart/{}?range={range}&interval={interval}",
            self.base_url,
            percent_encode(ticker)
        )
    }

    async fn fetch(&self, ticker: &str, range: &str, interval: &str) -> Result<Vec<u8>, DataError> {
        // Yahoo answers fine to ordinary requests; no special headers.
        let url = self.chart_url(ticker, range, interval);
        Ok(self
            .http
            .get(&url, &[("Accept", "application/json")])
            .await?
            .body)
    }
}

#[async_trait]
impl DataProvider for YahooProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Yahoo
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::QUOTE | Capabilities::HISTORY | Capabilities::INDEX
    }

    fn feed_kind(&self) -> FeedKind {
        FeedKind::Delayed
    }

    async fn quotes(&self, ids: &[InstrumentId]) -> Result<Vec<Sourced<Quote>>, DataError> {
        let mut set = JoinSet::new();
        for (i, id) in ids.iter().enumerate() {
            let this = (self.http.clone(), self.base_url.clone());
            let id = id.clone();
            set.spawn(async move {
                let provider = YahooProvider {
                    http: this.0,
                    base_url: this.1,
                };
                let result = async {
                    let body = provider.fetch(&equity_ticker(&id), "1d", "1d").await?;
                    parse_quote(&body, &id)
                }
                .await;
                (i, result)
            });
        }
        let mut results: Vec<(usize, Result<Sourced<Quote>, DataError>)> = Vec::new();
        while let Some(joined) = set.join_next().await {
            results.push(joined.map_err(|e| DataError::Upstream(format!("task failed: {e}")))?);
        }
        results.sort_by_key(|(i, _)| *i);
        settle(results.into_iter().map(|(_, r)| r).collect())
    }

    async fn history(&self, req: &HistoryRequest) -> Result<Sourced<CandleSeries>, DataError> {
        check_limits(req.interval, req.range)?;
        let body = self
            .fetch(
                &equity_ticker(&req.id),
                yahoo_range(req.range),
                yahoo_interval(req.interval),
            )
            .await?;
        parse_history(&body, req)
    }

    async fn index_snapshot(&self, index: IndexId) -> Result<Sourced<IndexSnapshot>, DataError> {
        let ticker = index_ticker(index).ok_or(DataError::Unsupported)?;
        let body = self.fetch(ticker, "1d", "1d").await?;
        // Yahoo has the level only, no constituents.
        Ok(parse_index_quote(&body, index)?.map(|quote| IndexSnapshot {
            quote,
            constituents: Vec::new(),
            constituents_info: None,
        }))
    }

    async fn all_indices(&self) -> Result<Vec<Sourced<IndexQuote>>, DataError> {
        // Probe one index first. If Yahoo is blocking us, rate-limiting us or
        // unreachable, say so now instead of sending twelve more requests that
        // will fail the same way (and, when throttled, make it worse).
        let (probe_index, probe_ticker) = INDEX_TICKERS[0];
        let probe = async {
            let body = self.fetch(probe_ticker, "1d", "1d").await?;
            parse_index_quote(&body, probe_index)
        }
        .await;
        match &probe {
            Err(DataError::NotFound) | Ok(_) => {}
            Err(other) => return Err(other.clone()),
        }
        let provider = Arc::new(YahooProvider {
            http: self.http.clone(),
            base_url: self.base_url.clone(),
        });
        let mut set = JoinSet::new();
        for (index, ticker) in INDEX_TICKERS.iter().skip(1) {
            let (provider, index, ticker) = (Arc::clone(&provider), *index, *ticker);
            set.spawn(async move {
                let result = async {
                    let body = provider.fetch(ticker, "1d", "1d").await?;
                    parse_index_quote(&body, index)
                }
                .await;
                (index, result)
            });
        }
        let mut results = vec![(probe_index, probe)];
        while let Some(joined) = set.join_next().await {
            let (index, result) =
                joined.map_err(|e| DataError::Upstream(format!("task failed: {e}")))?;
            if let Err(e) = &result {
                tracing::debug!(index = index.code(), error = %e, "yahoo index unavailable");
            }
            results.push((index, result));
        }
        // Dashboard order, not completion order.
        results.sort_by_key(|(index, _)| *index);
        settle(results.into_iter().map(|(_, r)| r).collect())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::http::{HostLimits, HttpConfig, Limit};

    const QUOTE_RELIANCE: &[u8] =
        include_bytes!("../tests/fixtures/yahoo/chart_quote_reliance.json");
    const QUOTE_NSEI: &[u8] = include_bytes!("../tests/fixtures/yahoo/chart_quote_nsei.json");
    const HISTORY: &[u8] = include_bytes!("../tests/fixtures/yahoo/chart_history_reliance_1d.json");
    const NOT_FOUND: &[u8] = include_bytes!("../tests/fixtures/yahoo/chart_not_found.json");
    const BAD_RANGE: &[u8] = include_bytes!("../tests/fixtures/yahoo/chart_bad_range.json");
    const MISSING_PRICE: &[u8] = include_bytes!("../tests/fixtures/yahoo/chart_missing_price.json");

    fn id(s: &str) -> InstrumentId {
        InstrumentId::parse(s).unwrap()
    }

    fn provider_for(server: &MockServer) -> YahooProvider {
        let limits = Arc::new(HostLimits::new(Limit::new(Duration::from_millis(1), 1000)));
        let config = HttpConfig {
            backoff_base: Duration::from_millis(1),
            ..HttpConfig::default()
        };
        YahooProvider::with_base_url(HttpClient::new(config, limits).unwrap(), &server.uri())
    }

    #[test]
    fn tickers_map_exchanges_and_encode_specials() {
        assert_eq!(equity_ticker(&id("RELIANCE")), "RELIANCE.NS");
        assert_eq!(equity_ticker(&id("BSE:500325")), "500325.BO");
    }

    #[test]
    fn every_catalogued_index_has_exactly_one_ticker() {
        for index in IndexId::all() {
            assert_eq!(
                INDEX_TICKERS.iter().filter(|(i, _)| *i == index).count(),
                1,
                "{index:?}"
            );
        }
        assert_eq!(index_ticker(IndexId::Nifty50), Some("^NSEI"));
    }

    #[test]
    fn parses_a_stock_quote() {
        let q = parse_quote(QUOTE_RELIANCE, &id("RELIANCE")).unwrap();
        assert_eq!(q.data.ltp, 1359.3);
        // The real response has chartPreviousClose but no previousClose.
        assert_eq!(q.data.prev_close, 1367.0);
        assert_eq!(q.data.open, Some(1370.0));
        assert_eq!(q.data.high, Some(1373.0));
        assert_eq!(q.data.low, Some(1354.5));
        assert_eq!(q.data.volume, Some(5_909_374));
        assert_eq!(q.data.week52_high, Some(1611.8));
        assert_eq!(q.data.name.as_deref(), Some("Reliance Industries Limited"));
        assert_eq!(q.data.upper_band, None);
        assert_eq!(q.provenance.provider, ProviderId::Yahoo);
        assert_eq!(q.provenance.feed, FeedKind::Delayed);
        assert_eq!(q.provenance.as_of.timestamp(), 1_794_980_393);
        assert!((q.data.change_pct() - -0.563).abs() < 0.01);
    }

    #[test]
    fn index_volume_of_zero_is_unknown_not_zero() {
        let q = parse_quote(QUOTE_NSEI, &id("NIFTY")).unwrap();
        assert_eq!(q.data.volume, None);
        let i = parse_index_quote(QUOTE_NSEI, IndexId::Nifty50).unwrap();
        assert_eq!(i.data.last, 25243.4);
        assert_eq!(i.data.advances, None);
        assert_eq!(i.data.week52_low, Some(22179.9));
    }

    #[test]
    fn parses_recorded_history() {
        let req = HistoryRequest {
            id: id("RELIANCE"),
            interval: Interval::D1,
            range: HistoryRange::M1,
        };
        let s = parse_history(HISTORY, &req).unwrap();
        assert_eq!(s.data.candles.len(), 21);
        assert!(s.data.is_ordered());
        assert_eq!(s.data.candles[0].t, 1_792_381_500_000);
        assert_eq!(s.provenance.as_of.timestamp(), 1_794_973_500);
        assert!(
            s.data
                .candles
                .iter()
                .all(|c| c.l <= c.o && c.o <= c.h && c.l <= c.c && c.c <= c.h)
        );
    }

    #[test]
    fn history_skips_null_rows() {
        let body = br#"{"chart":{"result":[{"meta":{},"timestamp":[10,20,30],"indicators":{"quote":[{"open":[1.0,null,3.0],"high":[2.0,null,4.0],"low":[0.5,null,2.5],"close":[1.5,null,3.5],"volume":[100,null,300]}]}}],"error":null}}"#;
        let req = HistoryRequest {
            id: id("X"),
            interval: Interval::D1,
            range: HistoryRange::D5,
        };
        let s = parse_history(body, &req).unwrap();
        let ts: Vec<_> = s.data.candles.iter().map(|c| c.t).collect();
        assert_eq!(ts, [10_000, 30_000]);
        assert_eq!(s.data.candles[1].v, 300.0);
    }

    #[test]
    fn error_bodies_map_to_typed_errors() {
        assert_eq!(
            parse_quote(NOT_FOUND, &id("ZZZ")).unwrap_err(),
            DataError::NotFound
        );
        assert!(
            matches!(parse_quote(BAD_RANGE, &id("X")).unwrap_err(), DataError::UnsupportedRange(m) if m.contains("60 days"))
        );
        assert!(matches!(
            parse_quote(MISSING_PRICE, &id("X")).unwrap_err(),
            DataError::Parse(_)
        ));
        assert!(matches!(
            parse_quote(b"<html>", &id("X")).unwrap_err(),
            DataError::Parse(_)
        ));
        assert!(matches!(
            parse_quote(br#"{"chart":{"result":[],"error":null}}"#, &id("X")).unwrap_err(),
            DataError::Parse(_)
        ));
    }

    #[test]
    fn mismatched_history_columns_are_a_parse_error() {
        let body = br#"{"chart":{"result":[{"meta":{},"timestamp":[1,2],"indicators":{"quote":[{"open":[1],"high":[1,1],"low":[1,1],"close":[1,1],"volume":[1,1]}]}}],"error":null}}"#;
        let req = HistoryRequest {
            id: id("X"),
            interval: Interval::D1,
            range: HistoryRange::D5,
        };
        assert!(matches!(
            parse_history(body, &req),
            Err(DataError::Parse(_))
        ));
    }

    #[test]
    fn interval_limits_are_typed_errors() {
        use HistoryRange::*;
        let cases = [
            (Interval::M1, D5, true),
            (Interval::M1, M1, false),
            (Interval::M5, M1, true),
            (Interval::M5, M3, false),
            (Interval::M15, Y1, false),
            (Interval::H1, Y1, true),
            (Interval::H1, Y5, false),
            (Interval::D1, Max, true),
            (Interval::W1, Max, true),
        ];
        for (interval, range, ok) in cases {
            let r = check_limits(interval, range);
            assert_eq!(r.is_ok(), ok, "{interval} over {range:?}: {r:?}");
            if let Err(e) = r {
                assert!(matches!(e, DataError::UnsupportedRange(_)));
                assert!(!e.counts_against_health());
            }
        }
    }

    #[tokio::test]
    async fn history_beyond_limits_makes_no_request() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let req = HistoryRequest {
            id: id("TCS"),
            interval: Interval::M1,
            range: HistoryRange::M3,
        };
        assert!(matches!(
            provider_for(&server).history(&req).await,
            Err(DataError::UnsupportedRange(_))
        ));
    }

    #[tokio::test]
    async fn fetches_quotes_concurrently_and_keeps_request_order() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v8/finance/chart/RELIANCE.NS"))
            .and(query_param("range", "1d"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(QUOTE_RELIANCE))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v8/finance/chart/ZZZ.NS"))
            .respond_with(ResponseTemplate::new(404).set_body_bytes(NOT_FOUND))
            .mount(&server)
            .await;
        let got = provider_for(&server)
            .quotes(&[id("ZZZ"), id("RELIANCE")])
            .await
            .unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].data.id, id("RELIANCE"));
    }

    #[tokio::test]
    async fn all_unknown_symbols_is_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404).set_body_bytes(NOT_FOUND))
            .mount(&server)
            .await;
        assert_eq!(
            provider_for(&server)
                .quotes(&[id("ZZZ")])
                .await
                .unwrap_err(),
            DataError::NotFound
        );
    }

    #[tokio::test]
    async fn rate_limiting_is_reported_when_nothing_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "600"))
            .mount(&server)
            .await;
        let err = provider_for(&server)
            .quotes(&[id("TCS")])
            .await
            .unwrap_err();
        assert!(matches!(err, DataError::RateLimited { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn index_snapshot_has_level_but_no_constituents() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v8/finance/chart/%5ENSEI"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(QUOTE_NSEI))
            .mount(&server)
            .await;
        let snap = provider_for(&server)
            .index_snapshot(IndexId::Nifty50)
            .await
            .unwrap();
        assert_eq!(snap.data.quote.last, 25243.4);
        assert!(snap.data.constituents.is_empty());
        assert_eq!(snap.provenance.provider, ProviderId::Yahoo);
    }

    #[tokio::test]
    async fn all_indices_gives_up_after_one_request_when_yahoo_is_refusing() {
        let server = MockServer::start().await;
        // Exactly one request, not thirteen.
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            provider_for(&server).all_indices().await.unwrap_err(),
            DataError::Blocked
        );
    }

    #[tokio::test]
    async fn all_indices_skips_failures_and_keeps_dashboard_order() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v8/finance/chart/%5ENSEI"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(QUOTE_NSEI))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v8/finance/chart/%5ENSEBANK"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(QUOTE_NSEI))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404).set_body_bytes(NOT_FOUND))
            .mount(&server)
            .await;
        let all = provider_for(&server).all_indices().await.unwrap();
        let ids: Vec<_> = all.iter().map(|q| q.data.index).collect();
        assert_eq!(ids, [IndexId::Nifty50, IndexId::NiftyBank]);
    }

    /// Live check against the real service. Run by hand:
    /// `cargo test -p nivesh-data -- --ignored --nocapture yahoo_live`.
    /// Never runs in CI. It makes a handful of polite requests.
    #[tokio::test]
    #[ignore = "hits the real Yahoo Finance"]
    async fn yahoo_live_smoke() {
        let limits = Arc::new(HostLimits::defaults());
        let yahoo = YahooProvider::new(HttpClient::new(HttpConfig::default(), limits).unwrap());
        let quotes = yahoo
            .quotes(&[id("RELIANCE")])
            .await
            .expect("RELIANCE quote");
        let q = &quotes[0];
        println!(
            "RELIANCE ltp={} prev={} as_of={}",
            q.data.ltp, q.data.prev_close, q.provenance.as_of
        );
        assert!(q.data.ltp > 0.0);

        let nifty = yahoo
            .index_snapshot(IndexId::Nifty50)
            .await
            .expect("NIFTY 50");
        println!(
            "NIFTY 50 last={} as_of={}",
            nifty.data.quote.last, nifty.provenance.as_of
        );
        assert!(nifty.data.quote.last > 1000.0);

        // Which catalogued sector tickers answer on Yahoo today.
        for (index, ticker) in INDEX_TICKERS {
            match yahoo.index_snapshot(*index).await {
                Ok(s) => println!("  ok   {ticker:<22} {}", s.data.quote.last),
                Err(e) => println!("  FAIL {ticker:<22} {e}"),
            }
        }
        let req = HistoryRequest {
            id: id("RELIANCE"),
            interval: Interval::D1,
            range: HistoryRange::M1,
        };
        let h = yahoo.history(&req).await.expect("history");
        println!("history: {} daily candles", h.data.candles.len());
        assert!(h.data.candles.len() > 10);
    }

    /// Maintainer tool: records raw chart responses as fixtures.
    /// `NIVESH_RECORD_DIR=crates/nivesh-data/tests/fixtures/yahoo cargo test -p
    /// nivesh-data -- --ignored yahoo_record`. Bodies hold only public market
    /// data; no headers or cookies are saved.
    #[tokio::test]
    #[ignore = "hits the real Yahoo Finance and writes files"]
    async fn yahoo_record_fixtures() {
        let Ok(dir) = std::env::var("NIVESH_RECORD_DIR") else {
            panic!("set NIVESH_RECORD_DIR to the fixtures directory");
        };
        let client = reqwest::Client::builder()
            .user_agent(crate::http::BROWSER_USER_AGENT)
            .build()
            .unwrap();
        let jobs = [
            ("chart_quote_reliance.json", "RELIANCE.NS", "1d", "1d"),
            ("chart_quote_nsei.json", "%5ENSEI", "1d", "1d"),
            ("chart_history_reliance_1d.json", "RELIANCE.NS", "1mo", "1d"),
            ("chart_not_found.json", "NOSUCHSYMBOLZZ.NS", "1d", "1d"),
            ("chart_bad_range.json", "RELIANCE.NS", "3mo", "5m"),
        ];
        for (file, ticker, range, interval) in jobs {
            let url = format!(
                "{DEFAULT_BASE_URL}/v8/finance/chart/{ticker}?range={range}&interval={interval}"
            );
            let resp = client.get(&url).send().await.unwrap();
            let status = resp.status();
            let body = resp.bytes().await.unwrap();
            println!("{file}: HTTP {status}, {} bytes", body.len());
            std::fs::write(std::path::Path::new(&dir).join(file), &body).unwrap();
            tokio::time::sleep(Duration::from_millis(1200)).await;
        }
    }
}
