//! NSE public-endpoint provider (`www.nseindia.com/api/...`).
//!
//! These are the undocumented JSON endpoints behind NSE's own website. They
//! need ordinary browser headers and the cookies set by loading a normal NSE
//! page first, and they change without notice, so every response goes through
//! a tolerant parser with fixtures.
//!
//! **We do not try to defeat blocking.** On 401/403 the session is
//! re-bootstrapped once; if that fails too the provider backs off (answering
//! `Blocked` without sending anything) and the chain falls back to another
//! provider. No proxies, no IP rotation, no fingerprint games.
//!
//! Everything is [`FeedKind::Delayed`]: NSE's website data may look close to
//! real time but is unofficial and carries no timeliness guarantee.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone as _, Utc};
use nivesh_core::{
    CandleSeries, Exchange, FeedKind, IndexId, IndexQuote, IndexSnapshot, Instrument, InstrumentId,
    Provenance, ProviderId, Quote, Sourced,
};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::http::HttpClient;
use crate::util::{percent_encode, settle};
use crate::{Capabilities, DataError, DataProvider, HistoryRequest};

pub const DEFAULT_BASE_URL: &str = "https://www.nseindia.com";
/// Static files (equity list) live on a separate archive host.
pub const DEFAULT_ARCHIVE_URL: &str = "https://nsearchives.nseindia.com";

/// NSE has no batch quote endpoint, so each symbol costs a request against a
/// roughly 1 request/second budget. Larger batches are declined (`Unsupported`)
/// so the chain sends them to a provider that copes better.
pub const MAX_QUOTES_PER_CALL: usize = 4;

#[derive(Debug, Clone)]
pub struct NseConfig {
    /// How long a warmed-up session is trusted before warming up again.
    pub session_ttl: Duration,
    /// First back-off after a refused session; doubles per repeat.
    pub block_backoff: Duration,
    pub block_backoff_max: Duration,
}

impl Default for NseConfig {
    fn default() -> Self {
        Self {
            session_ttl: Duration::from_secs(240),
            block_backoff: Duration::from_secs(60),
            block_backoff_max: Duration::from_secs(900),
        }
    }
}

/// Back-off state for one endpoint. NSE refuses per endpoint (and per IP), so
/// one refused endpoint doesn't silence the ones that still answer.
#[derive(Default, Clone, Copy)]
struct Backoff {
    until: Option<Instant>,
    strikes: u32,
}

#[derive(Default)]
struct Session {
    warmed_at: Option<Instant>,
    backoff: HashMap<String, Backoff>,
}

/// `quote-equity?symbol=X` -> `quote-equity`.
fn endpoint_of(path: &str) -> &str {
    path.split('?').next().unwrap_or(path)
}

pub struct NseProvider {
    http: HttpClient,
    base_url: String,
    archive_url: String,
    config: NseConfig,
    session: Mutex<Session>,
}

/// NSE's market-status summary for the capital market.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NseMarketStatus {
    pub open: bool,
    pub message: String,
    pub as_of: Option<DateTime<Utc>>,
}

impl NseProvider {
    /// `http` must have cookies enabled and should be dedicated to NSE.
    pub fn new(http: HttpClient) -> Self {
        Self::with_base_url(http, DEFAULT_BASE_URL, NseConfig::default())
    }

    pub fn with_base_url(http: HttpClient, base_url: &str, config: NseConfig) -> Self {
        Self {
            http,
            base_url: base_url.trim_end_matches('/').to_owned(),
            archive_url: DEFAULT_ARCHIVE_URL.to_owned(),
            config,
            session: Mutex::new(Session::default()),
        }
    }

    /// Points the equity-list download at another host (tests, local servers).
    pub fn with_archive_url(mut self, archive_url: &str) -> Self {
        self.archive_url = archive_url.trim_end_matches('/').to_owned();
        self
    }

    /// Warms the session by loading the home page, as a browser would. A
    /// failed warm-up is not fatal: the API call decides whether we are let in.
    async fn warm_up(&self, session: &mut Session) {
        let url = format!("{}/", self.base_url);
        let headers = [
            (
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            ),
            ("Accept-Language", "en-US,en;q=0.9"),
        ];
        if let Err(err) = self.http.get(&url, &headers).await {
            tracing::debug!(error = %err, "nse session warm-up failed");
        }
        session.warmed_at = Some(Instant::now());
    }

    async fn ensure_warm(&self, force: bool) {
        let mut session = self.session.lock().await;
        let fresh = session
            .warmed_at
            .is_some_and(|t| t.elapsed() < self.config.session_ttl);
        if force || !fresh {
            self.warm_up(&mut session).await;
        }
    }

    async fn check_not_blocked(&self, endpoint: &str) -> Result<(), DataError> {
        let session = self.session.lock().await;
        match session.backoff.get(endpoint).and_then(|b| b.until) {
            Some(until) if Instant::now() < until => Err(DataError::Blocked),
            _ => Ok(()),
        }
    }

    async fn record_block(&self, endpoint: &str) {
        let mut session = self.session.lock().await;
        let entry = session.backoff.entry(endpoint.to_owned()).or_default();
        entry.strikes = entry.strikes.saturating_add(1);
        let factor = 1u32 << (entry.strikes - 1).min(10);
        let wait = self
            .config
            .block_backoff
            .saturating_mul(factor)
            .min(self.config.block_backoff_max);
        entry.until = Some(Instant::now() + wait);
        tracing::warn!(
            endpoint,
            backoff_s = wait.as_secs(),
            "nse refused the session; backing off"
        );
    }

    async fn record_success(&self, endpoint: &str) {
        self.session.lock().await.backoff.remove(endpoint);
    }

    async fn raw_api_get(&self, path: &str) -> Result<Value, DataError> {
        let url = format!("{}/api/{path}", self.base_url);
        let referer = format!("{}/", self.base_url);
        let headers = [
            ("Accept", "application/json, text/plain, */*"),
            ("Accept-Language", "en-US,en;q=0.9"),
            ("Referer", referer.as_str()),
        ];
        self.http.get(&url, &headers).await?.json()
    }

    /// GET `/api/<path>` with session handling: warm up if needed, and on
    /// 401/403 re-bootstrap exactly once before backing off.
    async fn api_get(&self, path: &str) -> Result<Value, DataError> {
        let endpoint = endpoint_of(path);
        self.check_not_blocked(endpoint).await?;
        self.ensure_warm(false).await;
        let first = self.raw_api_get(path).await;
        let result = match first {
            Err(DataError::Unauthorized | DataError::Blocked) => {
                self.ensure_warm(true).await;
                match self.raw_api_get(path).await {
                    Err(DataError::Unauthorized | DataError::Blocked) => {
                        self.record_block(endpoint).await;
                        return Err(DataError::Blocked);
                    }
                    other => other,
                }
            }
            other => other,
        };
        if result.is_ok() {
            self.record_success(endpoint).await;
        }
        result
    }

    async fn quote_one(&self, id: &InstrumentId) -> Result<Sourced<Quote>, DataError> {
        let body = self
            .api_get(&format!(
                "quote-equity?symbol={}",
                percent_encode(id.symbol())
            ))
            .await?;
        parse_quote(&body, id)
    }

    /// The capital market's state, from `/api/marketStatus`.
    pub async fn market_status(&self) -> Result<NseMarketStatus, DataError> {
        parse_market_status(&self.api_get("marketStatus").await?)
    }
}

#[async_trait]
impl DataProvider for NseProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Nse
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::QUOTE | Capabilities::INDEX | Capabilities::INSTRUMENTS
    }

    fn feed_kind(&self) -> FeedKind {
        FeedKind::Delayed
    }

    async fn quotes(&self, ids: &[InstrumentId]) -> Result<Vec<Sourced<Quote>>, DataError> {
        let nse: Vec<&InstrumentId> = ids
            .iter()
            .filter(|i| i.exchange() == Exchange::Nse)
            .collect();
        if nse.is_empty() || nse.len() > MAX_QUOTES_PER_CALL {
            return Err(DataError::Unsupported);
        }
        let mut results = Vec::with_capacity(nse.len());
        for id in nse {
            let result = self.quote_one(id).await;
            // A refused session won't improve for the next symbol.
            if let Err(DataError::Blocked) = result {
                return Err(DataError::Blocked);
            }
            results.push(result);
        }
        settle(results)
    }

    async fn history(&self, _req: &HistoryRequest) -> Result<Sourced<CandleSeries>, DataError> {
        Err(DataError::Unsupported)
    }

    async fn index_snapshot(&self, index: IndexId) -> Result<Sourced<IndexSnapshot>, DataError> {
        let name = index.nse_name().ok_or(DataError::Unsupported)?;
        let body = self
            .api_get(&format!(
                "equity-stockIndices?index={}",
                percent_encode(name)
            ))
            .await?;
        parse_stock_indices(&body, index)
    }

    async fn all_indices(&self) -> Result<Vec<Sourced<IndexQuote>>, DataError> {
        parse_all_indices(&self.api_get("allIndices").await?)
    }

    /// NSE's list of equities. A static file on the archive host: it needs no
    /// session, and is fetched at most once a day.
    async fn instruments(&self, exchange: Exchange) -> Result<Vec<Instrument>, DataError> {
        if exchange != Exchange::Nse {
            return Err(DataError::Unsupported);
        }
        let url = format!("{}/content/equities/EQUITY_L.csv", self.archive_url);
        let response = self.http.get(&url, &[("Accept", "text/csv,*/*")]).await?;
        crate::instruments::parse_equity_csv(&response.text())
    }
}

// ---- parsing --------------------------------------------------------------

/// NSE writes numbers as JSON numbers, as strings (`"1,234.50"`), or as `"-"`
/// for "none". Anything unusable is `None`.
fn num(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let cleaned: String = s.chars().filter(|c| *c != ',').collect();
            cleaned.trim().parse().ok()
        }
        _ => None,
    }
}

fn field(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(num).filter(|v| v.is_finite())
}

/// A positive price: zero is how NSE says "not applicable".
fn price(value: &Value, key: &str) -> Option<f64> {
    field(value, key).filter(|v| *v > 0.0)
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_owned)
}

fn count(value: &Value, key: &str) -> Option<u32> {
    let n = value.get(key).and_then(num)?;
    (n >= 0.0 && n <= f64::from(u32::MAX)).then_some(n as u32)
}

/// Parses NSE timestamps such as `18-Nov-2026 15:30:00` or
/// `18-Nov-2026 11:10`, which are in IST.
pub fn parse_nse_time(text: &str) -> Result<DateTime<Utc>, DataError> {
    let text = text.trim();
    let naive = ["%d-%b-%Y %H:%M:%S", "%d-%b-%Y %H:%M"]
        .iter()
        .find_map(|fmt| NaiveDateTime::parse_from_str(text, fmt).ok())
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(text, "%d-%b-%Y")
                .ok()
                .and_then(|d| d.and_hms_opt(15, 30, 0))
        })
        .ok_or_else(|| DataError::Parse(format!("nse: bad timestamp `{text}`")))?;
    // IST is a fixed +05:30 with no DST.
    FixedOffset::east_opt(5 * 3600 + 30 * 60)
        .ok_or_else(|| parse_err("IST offset out of range"))?
        .from_local_datetime(&naive)
        .single()
        .map(|t| t.with_timezone(&Utc))
        .ok_or_else(|| DataError::Parse(format!("nse: ambiguous timestamp `{text}`")))
}

fn provenance(as_of: DateTime<Utc>) -> Provenance {
    Provenance::new(ProviderId::Nse, FeedKind::Delayed, as_of, Utc::now())
}

fn parse_err(what: &str) -> DataError {
    DataError::Parse(format!("nse: {what}"))
}

/// `/api/quote-equity?symbol=X`.
///
/// Volume is left unset: `tradeInfo.totalTradedVolume` is reported in a
/// different unit than the other endpoints and the unit could not be checked
/// against a live response, so a wrong number is worse than none.
pub fn parse_quote(body: &Value, id: &InstrumentId) -> Result<Sourced<Quote>, DataError> {
    let info = body
        .get("priceInfo")
        .ok_or_else(|| parse_err("quote has no priceInfo"))?;
    let ltp = price(info, "lastPrice").ok_or_else(|| parse_err("quote has no lastPrice"))?;
    let prev_close =
        price(info, "previousClose").ok_or_else(|| parse_err("quote has no previousClose"))?;
    let updated = body
        .pointer("/metadata/lastUpdateTime")
        .and_then(Value::as_str)
        .ok_or_else(|| parse_err("quote has no lastUpdateTime"))?;
    let day = info.get("intraDayHighLow").unwrap_or(&Value::Null);
    let year = info.get("weekHighLow").unwrap_or(&Value::Null);
    let quote = Quote {
        id: id.clone(),
        name: body
            .pointer("/info/companyName")
            .and_then(Value::as_str)
            .map(str::to_owned),
        ltp,
        prev_close,
        open: price(info, "open"),
        high: price(day, "max"),
        low: price(day, "min"),
        volume: None,
        week52_high: price(year, "max"),
        week52_low: price(year, "min"),
        upper_band: price(info, "upperCP"),
        lower_band: price(info, "lowerCP"),
    };
    Ok(Sourced::new(quote, provenance(parse_nse_time(updated)?)))
}

/// `/api/allIndices`: one row per NSE index. Catalogued indices NSE doesn't
/// carry (SENSEX) are simply absent; the chain fills them from elsewhere.
pub fn parse_all_indices(body: &Value) -> Result<Vec<Sourced<IndexQuote>>, DataError> {
    let rows = body
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| parse_err("allIndices has no data array"))?;
    let as_of = parse_nse_time(
        body.get("timestamp")
            .and_then(Value::as_str)
            .ok_or_else(|| parse_err("allIndices has no timestamp"))?,
    )?;
    let mut out = Vec::new();
    for index in IndexId::all() {
        let Some(name) = index.nse_name() else {
            continue;
        };
        let Some(row) = rows
            .iter()
            .find(|r| r.get("index").and_then(Value::as_str) == Some(name))
        else {
            continue;
        };
        // A row we can't read is skipped rather than failing the whole table.
        let (Some(last), Some(prev_close)) = (price(row, "last"), price(row, "previousClose"))
        else {
            continue;
        };
        out.push(Sourced::new(
            IndexQuote {
                index,
                last,
                prev_close,
                open: price(row, "open"),
                high: price(row, "high"),
                low: price(row, "low"),
                week52_high: price(row, "yearHigh"),
                week52_low: price(row, "yearLow"),
                advances: count(row, "advances"),
                declines: count(row, "declines"),
            },
            provenance(as_of),
        ));
    }
    if out.is_empty() {
        return Err(parse_err(
            "allIndices contained none of the catalogued indices",
        ));
    }
    Ok(out)
}

/// `/api/equity-stockIndices?index=NAME`: the index row plus its constituents.
pub fn parse_stock_indices(
    body: &Value,
    index: IndexId,
) -> Result<Sourced<IndexSnapshot>, DataError> {
    let rows = body
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| parse_err("stock indices has no data array"))?;
    let name = body.get("name").and_then(Value::as_str).unwrap_or_default();
    let as_of = parse_nse_time(
        body.get("timestamp")
            .and_then(Value::as_str)
            .ok_or_else(|| parse_err("stock indices has no timestamp"))?,
    )?;

    let is_index_row = |r: &Value| {
        r.get("priority").and_then(num) == Some(1.0)
            || r.get("symbol").and_then(Value::as_str) == Some(name)
    };
    let meta = body.get("metadata").unwrap_or(&Value::Null);
    let index_row = rows
        .iter()
        .find(|r| is_index_row(r))
        .unwrap_or(&Value::Null);

    let last = price(meta, "last").or_else(|| price(index_row, "lastPrice"));
    let prev_close = price(meta, "previousClose").or_else(|| price(index_row, "previousClose"));
    let (Some(last), Some(prev_close)) = (last, prev_close) else {
        return Err(parse_err("stock indices has no index level"));
    };
    let advance = body.get("advance").unwrap_or(&Value::Null);
    let quote = IndexQuote {
        index,
        last,
        prev_close,
        open: price(meta, "open").or_else(|| price(index_row, "open")),
        high: price(meta, "high").or_else(|| price(index_row, "dayHigh")),
        low: price(meta, "low").or_else(|| price(index_row, "dayLow")),
        week52_high: price(meta, "yearHigh").or_else(|| price(index_row, "yearHigh")),
        week52_low: price(meta, "yearLow").or_else(|| price(index_row, "yearLow")),
        advances: count(advance, "advances"),
        declines: count(advance, "declines"),
    };

    let mut constituents = Vec::new();
    for row in rows.iter().filter(|r| !is_index_row(r)) {
        let Some(symbol) = row.get("symbol").and_then(Value::as_str) else {
            continue;
        };
        let Ok(id) = InstrumentId::new(Exchange::Nse, symbol) else {
            continue;
        };
        let (Some(ltp), Some(prev)) = (price(row, "lastPrice"), price(row, "previousClose")) else {
            continue;
        };
        constituents.push(Quote {
            id,
            name: row
                .pointer("/meta/companyName")
                .and_then(Value::as_str)
                .map(str::to_owned),
            ltp,
            prev_close: prev,
            open: price(row, "open"),
            high: price(row, "dayHigh"),
            low: price(row, "dayLow"),
            volume: field(row, "totalTradedVolume")
                .filter(|v| *v >= 0.0)
                .map(|v| v as u64),
            week52_high: price(row, "yearHigh"),
            week52_low: price(row, "yearLow"),
            upper_band: None,
            lower_band: None,
        });
    }
    if constituents.is_empty() {
        return Err(parse_err("stock indices has no constituents"));
    }
    Ok(Sourced::new(
        IndexSnapshot {
            quote,
            constituents,
        },
        provenance(as_of),
    ))
}

/// `/api/marketStatus`.
pub fn parse_market_status(body: &Value) -> Result<NseMarketStatus, DataError> {
    let states = body
        .get("marketState")
        .and_then(Value::as_array)
        .ok_or_else(|| parse_err("marketStatus has no marketState"))?;
    let capital = states
        .iter()
        .find(|s| s.get("market").and_then(Value::as_str) == Some("Capital Market"))
        .ok_or_else(|| parse_err("marketStatus has no Capital Market"))?;
    Ok(NseMarketStatus {
        open: capital.get("marketStatus").and_then(Value::as_str) == Some("Open"),
        message: text(capital, "marketStatusMessage").unwrap_or_default(),
        as_of: text(capital, "tradeDate").and_then(|t| parse_nse_time(&t).ok()),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    use super::*;
    use crate::http::{HostLimits, HttpConfig, Limit};

    const ALL_INDICES: &str = include_str!("../tests/fixtures/nse/all_indices.json");
    const MARKET_STATUS: &str = include_str!("../tests/fixtures/nse/market_status.json");
    const QUOTE: &str = include_str!("../tests/fixtures/nse/quote_equity_reliance.synthetic.json");
    const QUOTE_STRINGS: &str =
        include_str!("../tests/fixtures/nse/quote_equity_band_strings.synthetic.json");
    const STOCK_INDICES: &str =
        include_str!("../tests/fixtures/nse/stock_indices_nifty50.synthetic.json");

    fn json(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    fn id(s: &str) -> InstrumentId {
        InstrumentId::parse(s).unwrap()
    }

    #[test]
    fn nse_timestamps_are_ist() {
        let t = parse_nse_time("18-Nov-2026 15:30:00").unwrap();
        assert_eq!(t.to_rfc3339(), "2026-11-18T10:00:00+00:00");
        assert_eq!(
            parse_nse_time("18-Nov-2026 11:10").unwrap().to_rfc3339(),
            "2026-11-18T05:40:00+00:00"
        );
        assert_eq!(
            parse_nse_time("18-Nov-2026").unwrap().to_rfc3339(),
            "2026-11-18T10:00:00+00:00"
        );
        assert!(parse_nse_time("yesterday").is_err());
    }

    #[test]
    fn numbers_tolerate_strings_commas_and_dashes() {
        assert_eq!(num(&json("\"1,234.50\"")), Some(1234.5));
        assert_eq!(num(&json("12")), Some(12.0));
        assert_eq!(num(&json("\"-\"")), None);
        assert_eq!(num(&json("null")), None);
    }

    #[test]
    fn parses_a_quote_with_bands() {
        let q = parse_quote(&json(QUOTE), &id("RELIANCE")).unwrap();
        assert_eq!(q.data.ltp, 1359.3);
        assert_eq!(q.data.prev_close, 1367.0);
        assert_eq!(q.data.open, Some(1370.0));
        assert_eq!(q.data.high, Some(1373.0));
        assert_eq!(q.data.low, Some(1354.5));
        assert_eq!(q.data.week52_high, Some(1611.8));
        assert_eq!(q.data.upper_band, Some(1503.7));
        assert_eq!(q.data.lower_band, Some(1230.3));
        assert_eq!(
            q.data.volume, None,
            "volume unit is unverified, so it stays unset"
        );
        assert_eq!(q.data.name.as_deref(), Some("Reliance Industries Limited"));
        assert_eq!(q.provenance.provider, ProviderId::Nse);
        assert_eq!(q.provenance.feed, FeedKind::Delayed);
        assert_eq!(q.provenance.as_of.to_rfc3339(), "2026-11-18T05:44:07+00:00");
    }

    #[test]
    fn quote_strings_and_dash_bands() {
        let q = parse_quote(&json(QUOTE_STRINGS), &id("M&M")).unwrap();
        assert_eq!(q.data.ltp, 3150.5);
        assert_eq!(q.data.lower_band, None);
        assert_eq!(q.data.upper_band, Some(3454.0));
    }

    #[test]
    fn quote_missing_fields_are_parse_errors() {
        for body in [
            r#"{}"#,
            r#"{"priceInfo":{}}"#,
            r#"{"priceInfo":{"lastPrice":0,"previousClose":1},"metadata":{"lastUpdateTime":"18-Nov-2026 15:30:00"}}"#,
            r#"{"priceInfo":{"lastPrice":5,"previousClose":1}}"#,
        ] {
            assert!(
                matches!(parse_quote(&json(body), &id("X")), Err(DataError::Parse(_))),
                "{body}"
            );
        }
    }

    #[test]
    fn parses_recorded_all_indices() {
        let all = parse_all_indices(&json(ALL_INDICES)).unwrap();
        let found: Vec<_> = all.iter().map(|q| q.data.index).collect();
        // Sensex is not an NSE index; the other twelve are present.
        assert_eq!(found.len(), 12);
        assert!(!found.contains(&IndexId::Sensex));
        assert_eq!(found[0], IndexId::Nifty50, "dashboard order");
        let nifty = &all[0];
        assert_eq!(nifty.data.last, 25237.5);
        assert_eq!(nifty.data.prev_close, 25387.4);
        assert_eq!(nifty.data.advances, Some(10));
        assert_eq!(nifty.data.declines, Some(40));
        assert_eq!(nifty.data.week52_high, Some(26373.2));
        assert_eq!(
            nifty.provenance.as_of.to_rfc3339(),
            "2026-11-18T05:39:00+00:00"
        );
        assert_eq!(nifty.provenance.feed, FeedKind::Delayed);
        // NIFTY FINANCIAL SERVICES maps to the catalogue entry with that NSE name.
        assert!(found.contains(&IndexId::NiftyFinService));
    }

    #[test]
    fn all_indices_rejects_garbage() {
        assert!(parse_all_indices(&json("{}")).is_err());
        assert!(
            parse_all_indices(&json(r#"{"data":[],"timestamp":"18-Nov-2026 11:10"}"#)).is_err()
        );
    }

    #[test]
    fn parses_stock_indices_and_skips_bad_rows() {
        let snap = parse_stock_indices(&json(STOCK_INDICES), IndexId::Nifty50).unwrap();
        assert_eq!(snap.data.quote.last, 25243.4);
        assert_eq!(snap.data.quote.advances, Some(11));
        assert_eq!(snap.data.quote.declines, Some(39));
        let syms: Vec<_> = snap
            .data
            .constituents
            .iter()
            .map(|q| q.id.symbol())
            .collect();
        assert_eq!(
            syms,
            ["RELIANCE", "TCS"],
            "index row and unreadable row are not constituents"
        );
        assert_eq!(snap.data.constituents[0].volume, Some(5_909_374));
        assert_eq!(
            snap.data.constituents[1].name.as_deref(),
            Some("Tata Consultancy Services Limited")
        );
        assert_eq!(snap.provenance.provider, ProviderId::Nse);
    }

    #[test]
    fn parses_recorded_market_status() {
        let s = parse_market_status(&json(MARKET_STATUS)).unwrap();
        assert!(s.open);
        assert_eq!(s.message, "Normal Market is Open");
        assert_eq!(s.as_of.unwrap().to_rfc3339(), "2026-11-18T05:39:00+00:00");
        assert!(parse_market_status(&json("{}")).is_err());
    }

    // ---- session handling against a mock server ----

    fn provider_for(server: &MockServer, backoff: Duration) -> NseProvider {
        let limits = Arc::new(HostLimits::new(Limit::new(Duration::from_millis(1), 1000)));
        let config = HttpConfig {
            cookies: true,
            backoff_base: Duration::from_millis(1),
            ..HttpConfig::default()
        };
        let config_nse = NseConfig {
            block_backoff: backoff,
            block_backoff_max: backoff * 4,
            ..NseConfig::default()
        };
        NseProvider::with_base_url(
            HttpClient::new(config, limits).unwrap(),
            &server.uri(),
            config_nse,
        )
    }

    /// 403 until the session cookie has been set by the home page.
    async fn cookie_gated_server() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(
                ResponseTemplate::new(200).insert_header("Set-Cookie", "nsit=abc; Path=/"),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/quote-equity"))
            .and(header("cookie", "nsit=abc"))
            .and(header("referer", format!("{}/", server.uri()).as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_string(QUOTE))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/quote-equity"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn warms_up_the_session_then_sends_cookies_and_referer() {
        let server = cookie_gated_server().await;
        let nse = provider_for(&server, Duration::from_secs(60));
        let q = nse.quotes(&[id("RELIANCE")]).await.unwrap();
        assert_eq!(q[0].data.ltp, 1359.3);
    }

    struct FlipAfter(AtomicUsize);
    impl Respond for FlipAfter {
        fn respond(&self, _: &Request) -> ResponseTemplate {
            // First API call is refused (expired cookie); the retry after a
            // re-bootstrap succeeds.
            if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(401)
            } else {
                ResponseTemplate::new(200).set_body_string(QUOTE)
            }
        }
    }

    #[tokio::test]
    async fn rebootstraps_once_on_401_and_recovers() {
        let server = MockServer::start().await;
        // Exactly two warm-ups: the initial one and one re-bootstrap.
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/quote-equity"))
            .respond_with(FlipAfter(AtomicUsize::new(0)))
            .expect(2)
            .mount(&server)
            .await;
        let nse = provider_for(&server, Duration::from_secs(60));
        assert_eq!(nse.quotes(&[id("RELIANCE")]).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn persistent_403_rebootstraps_once_then_backs_off_and_reports_blocked() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200))
            .expect(2)
            .mount(&server)
            .await;
        // One try + one retry after the re-bootstrap; nothing afterwards.
        Mock::given(method("GET"))
            .and(path("/api/quote-equity"))
            .respond_with(ResponseTemplate::new(403))
            .expect(2)
            .mount(&server)
            .await;
        let nse = provider_for(&server, Duration::from_secs(60));
        assert_eq!(
            nse.quotes(&[id("RELIANCE")]).await.unwrap_err(),
            DataError::Blocked
        );
        // Backing off: no further requests are made at all (`expect` above).
        for _ in 0..3 {
            assert_eq!(
                nse.quotes(&[id("RELIANCE")]).await.unwrap_err(),
                DataError::Blocked
            );
        }
    }

    #[tokio::test]
    async fn a_refused_endpoint_does_not_silence_the_others() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/quote-equity"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/allIndices"))
            .respond_with(ResponseTemplate::new(200).set_body_string(ALL_INDICES))
            .mount(&server)
            .await;
        let nse = provider_for(&server, Duration::from_secs(60));
        assert_eq!(
            nse.quotes(&[id("RELIANCE")]).await.unwrap_err(),
            DataError::Blocked
        );
        assert_eq!(nse.all_indices().await.unwrap().len(), 12);
    }

    #[tokio::test]
    async fn backoff_expires_and_the_provider_tries_again() {
        let server = cookie_gated_server().await;
        let blocked = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&blocked)
            .await;
        // Same provider object, so point it at the failing server first.
        let nse = provider_for(&blocked, Duration::from_millis(120));
        assert_eq!(
            nse.quotes(&[id("TCS")]).await.unwrap_err(),
            DataError::Blocked
        );
        assert_eq!(
            nse.quotes(&[id("TCS")]).await.unwrap_err(),
            DataError::Blocked
        );
        tokio::time::sleep(Duration::from_millis(150)).await;
        // After the back-off it sends a request again (and is refused again).
        let before = blocked.received_requests().await.unwrap().len();
        assert_eq!(
            nse.quotes(&[id("TCS")]).await.unwrap_err(),
            DataError::Blocked
        );
        assert!(blocked.received_requests().await.unwrap().len() > before);
        drop(server);
    }

    #[tokio::test]
    async fn a_failed_warm_up_is_not_fatal() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/allIndices"))
            .respond_with(ResponseTemplate::new(200).set_body_string(ALL_INDICES))
            .mount(&server)
            .await;
        let all = provider_for(&server, Duration::from_secs(60))
            .all_indices()
            .await
            .unwrap();
        assert_eq!(all.len(), 12);
    }

    #[tokio::test]
    async fn index_snapshot_uses_the_nse_name_and_sensex_is_unsupported() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/equity-stockIndices"))
            .and(query_param("index", "NIFTY 50"))
            .respond_with(ResponseTemplate::new(200).set_body_string(STOCK_INDICES))
            .expect(1)
            .mount(&server)
            .await;
        let nse = provider_for(&server, Duration::from_secs(60));
        let snap = nse.index_snapshot(IndexId::Nifty50).await.unwrap();
        assert_eq!(snap.data.constituents.len(), 2);
        assert_eq!(
            nse.index_snapshot(IndexId::Sensex).await.unwrap_err(),
            DataError::Unsupported
        );
    }

    #[tokio::test]
    async fn oversized_and_non_nse_batches_are_declined_without_requests() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let nse = provider_for(&server, Duration::from_secs(60));
        let many: Vec<_> = (0..=MAX_QUOTES_PER_CALL)
            .map(|i| id(&format!("S{i}")))
            .collect();
        assert_eq!(nse.quotes(&many).await.unwrap_err(), DataError::Unsupported);
        assert_eq!(
            nse.quotes(&[id("BSE:500325")]).await.unwrap_err(),
            DataError::Unsupported
        );
        assert_eq!(nse.quotes(&[]).await.unwrap_err(), DataError::Unsupported);
    }

    #[tokio::test]
    async fn downloads_and_parses_the_equity_list_from_the_archive_host() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/content/equities/EQUITY_L.csv"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(include_str!("../tests/fixtures/nse/equity_list.csv")),
            )
            .expect(1)
            .mount(&server)
            .await;
        let nse = provider_for(&server, Duration::from_secs(60)).with_archive_url(&server.uri());
        let list = nse.instruments(Exchange::Nse).await.unwrap();
        assert_eq!(list.len(), 9);
        assert_eq!(
            nse.instruments(Exchange::Bse).await.unwrap_err(),
            DataError::Unsupported
        );
    }

    #[tokio::test]
    async fn an_html_page_with_status_200_is_a_parse_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/allIndices"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>challenge</html>"))
            .mount(&server)
            .await;
        let err = provider_for(&server, Duration::from_secs(60))
            .all_indices()
            .await
            .unwrap_err();
        assert!(matches!(err, DataError::Parse(_)), "{err:?}");
    }

    /// Live check against the real NSE. Run by hand:
    /// `cargo test -p nivesh-data -- --ignored --nocapture nse_live`.
    /// Never runs in CI (NSE blocks datacenter IPs). Reports each endpoint
    /// separately because NSE answers some and refuses others per IP.
    #[tokio::test]
    #[ignore = "hits the real NSE website"]
    async fn nse_live_smoke() {
        let limits = Arc::new(HostLimits::defaults());
        let http = HttpClient::new(
            HttpConfig {
                cookies: true,
                ..HttpConfig::default()
            },
            limits,
        )
        .unwrap();
        let nse = NseProvider::new(http);
        let mut ok = 0;
        match nse.market_status().await {
            Ok(s) => {
                ok += 1;
                println!(
                    "marketStatus: open={} {:?} as_of={:?}",
                    s.open, s.message, s.as_of
                )
            }
            Err(e) => println!("marketStatus: FAIL {e}"),
        }
        match nse.all_indices().await {
            Ok(all) => {
                ok += 1;
                for q in all.iter().take(3) {
                    println!(
                        "allIndices: {} {} (as_of {})",
                        q.data.index, q.data.last, q.provenance.as_of
                    )
                }
            }
            Err(e) => println!("allIndices: FAIL {e}"),
        }
        match nse.quotes(&[id("RELIANCE")]).await {
            Ok(q) => {
                ok += 1;
                println!(
                    "quote RELIANCE: {} (as_of {})",
                    q[0].data.ltp, q[0].provenance.as_of
                )
            }
            Err(e) => println!("quote RELIANCE: FAIL {e}"),
        }
        match nse.instruments(Exchange::Nse).await {
            Ok(list) => {
                ok += 1;
                println!("equity list: {} instruments", list.len())
            }
            Err(e) => println!("equity list: FAIL {e}"),
        }
        match nse.index_snapshot(IndexId::Nifty50).await {
            Ok(s) => {
                ok += 1;
                println!(
                    "NIFTY 50: {} with {} constituents",
                    s.data.quote.last,
                    s.data.constituents.len()
                )
            }
            Err(e) => println!("NIFTY 50 constituents: FAIL {e}"),
        }
        assert!(
            ok > 0,
            "NSE refused every endpoint (it blocks many networks)"
        );
    }
}
