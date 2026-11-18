//! BSE's equity list, from the public daily bhavcopy.
//!
//! BSE's scrip-master API (`api.bseindia.com`) answers 403 to ordinary clients,
//! and the plan treats it as unavailable. The daily bhavcopy file (UDiFF format)
//! on `www.bseindia.com` is a plain download that lists every scrip traded that
//! day with its ticker, scrip code, ISIN and name, which is all the instrument
//! master needs. It is fetched at most once a day, like NSE's list.
//!
//! This provider supplies instrument lists only. It is not part of the quote
//! routing: BSE quotes come from Yahoo's `.BO` tickers.

use async_trait::async_trait;
use chrono::{Datelike as _, Duration, Weekday};
use nivesh_core::{Exchange, FeedKind, Instrument, InstrumentId, ProviderId};

use crate::http::HttpClient;
use crate::instruments::split_csv_line;
use crate::{Capabilities, DataError, DataProvider};

pub const DEFAULT_BASE_URL: &str = "https://www.bseindia.com";

/// How many days back to look for the latest bhavcopy: a long weekend plus
/// holidays, and the file for today only appears in the evening.
const LOOKBACK_DAYS: usize = 8;

/// BSE groups that are not equity: `F` is fixed income and `G` government
/// securities.
const NON_EQUITY_SERIES: [&str; 2] = ["F", "G"];

/// Parses a bhavcopy into BSE equities: one row per scrip, keeping company
/// shares (ISIN `INE...`) and dropping debt groups, mutual-fund units and ETFs
/// (their tickers read like stock symbols and would pollute search). Columns
/// are found by header name.
pub fn parse_bhavcopy(text: &str) -> Result<Vec<Instrument>, DataError> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header = lines
        .next()
        .ok_or_else(|| DataError::Parse("bhavcopy is empty".into()))?;
    let names: Vec<String> = split_csv_line(header)
        .iter()
        .map(|h| h.trim().to_owned())
        .collect();
    let col = |wanted: &str| names.iter().position(|n| n == wanted);
    let (Some(sym), Some(name), Some(isin), Some(series)) = (
        col("TckrSymb"),
        col("FinInstrmNm"),
        col("ISIN"),
        col("SctySrs"),
    ) else {
        return Err(DataError::Parse(
            "bhavcopy has no TckrSymb / FinInstrmNm / ISIN / SctySrs columns".into(),
        ));
    };

    let mut out: Vec<Instrument> = Vec::new();
    for line in lines {
        let f = split_csv_line(line);
        let get = |i: usize| f.get(i).map(|v| v.trim()).filter(|v| !v.is_empty());
        let (Some(symbol), Some(name), Some(isin), Some(series)) =
            (get(sym), get(name), get(isin), get(series))
        else {
            continue;
        };
        if !isin.starts_with("INE") || NON_EQUITY_SERIES.contains(&series) {
            continue;
        }
        let Ok(id) = InstrumentId::new(Exchange::Bse, symbol) else {
            continue;
        };
        if out.iter().any(|i| i.id == id) {
            continue;
        }
        out.push(Instrument {
            id,
            name: name.to_owned(),
            isin: Some(isin.to_owned()),
            series: Some(series.to_owned()),
        });
    }
    if out.is_empty() {
        return Err(DataError::Parse("bhavcopy has no equity rows".into()));
    }
    Ok(out)
}

/// `YYYYMMDD` of recent weekdays in India, newest first, starting from the day
/// `now_ms` falls on.
pub fn candidate_dates(now_ms: i64) -> Vec<String> {
    const IST_OFFSET_MS: i64 = 330 * 60 * 1000;
    let Some(now) = chrono::DateTime::from_timestamp_millis(now_ms + IST_OFFSET_MS) else {
        return Vec::new();
    };
    let today = now.date_naive();
    (0..i64::try_from(LOOKBACK_DAYS).unwrap_or(8))
        .map(|back| today - Duration::days(back))
        .filter(|d| !matches!(d.weekday(), Weekday::Sat | Weekday::Sun))
        .map(|d| d.format("%Y%m%d").to_string())
        .collect()
}

pub struct BseListProvider {
    http: HttpClient,
    base_url: String,
    now_ms: fn() -> i64,
}

fn system_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or_default()
}

impl BseListProvider {
    pub fn new(http: HttpClient) -> Self {
        Self::with_base_url(http, DEFAULT_BASE_URL)
    }

    /// For tests and local fixture servers.
    pub fn with_base_url(http: HttpClient, base_url: &str) -> Self {
        Self {
            http,
            base_url: base_url.trim_end_matches('/').to_owned(),
            now_ms: system_now_ms,
        }
    }

    /// Tests pin the date so the files they serve are the ones asked for.
    pub fn with_clock(mut self, now_ms: fn() -> i64) -> Self {
        self.now_ms = now_ms;
        self
    }
}

#[async_trait]
impl DataProvider for BseListProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Bse
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::INSTRUMENTS
    }

    fn feed_kind(&self) -> FeedKind {
        FeedKind::Delayed
    }

    /// The newest bhavcopy that exists. A non-trading day gets an HTML page or
    /// a 404 instead of a file; that moves on to the previous day.
    async fn instruments(&self, exchange: Exchange) -> Result<Vec<Instrument>, DataError> {
        if exchange != Exchange::Bse {
            return Err(DataError::Unsupported);
        }
        let mut last_err = DataError::NotFound;
        for date in candidate_dates((self.now_ms)()) {
            let url = format!(
                "{}/download/BhavCopy/Equity/BhavCopy_BSE_CM_0_0_0_{date}_F_0000.CSV",
                self.base_url
            );
            let attempt = match self.http.get(&url, &[("Accept", "text/csv,*/*")]).await {
                Ok(response) => parse_bhavcopy(&response.text()),
                Err(e) => Err(e),
            };
            match attempt {
                Ok(list) => return Ok(list),
                // Blocked or throttled: asking for more days won't help.
                Err(e @ (DataError::Blocked | DataError::RateLimited { .. })) => return Err(e),
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration as StdDuration;

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::http::{HostLimits, HttpConfig, Limit};

    const FIXTURE: &str = include_str!("../tests/fixtures/bse/bhavcopy_sample.csv");

    fn provider(server: &MockServer) -> BseListProvider {
        let limits = Arc::new(HostLimits::new(Limit::new(
            StdDuration::from_millis(1),
            100,
        )));
        let config = HttpConfig {
            max_attempts: 1,
            ..HttpConfig::default()
        };
        BseListProvider::with_base_url(HttpClient::new(config, limits).unwrap(), &server.uri())
            // 2026-11-18 10:00 IST, a Wednesday.
            .with_clock(|| 1_794_996_000_000)
    }

    fn syms(list: &[Instrument]) -> Vec<&str> {
        list.iter().map(|i| i.id.symbol()).collect()
    }

    #[test]
    fn keeps_company_shares_and_drops_debt_funds_and_duplicates() {
        let list = parse_bhavcopy(FIXTURE).unwrap();
        assert_eq!(syms(&list), ["ABB", "RELIANCE", "SAMOR", "TCS", "ARE&M"]);
        assert!(list.iter().all(|i| i.id.exchange() == Exchange::Bse));
        let reliance = list.iter().find(|i| i.id.symbol() == "RELIANCE").unwrap();
        assert_eq!(reliance.name, "RELIANCE INDUSTRIES LTD.");
        assert_eq!(reliance.isin.as_deref(), Some("INE002A01018"));
        assert_eq!(reliance.series.as_deref(), Some("A"));
    }

    #[test]
    fn rejects_html_and_files_without_the_columns() {
        assert!(parse_bhavcopy("").is_err());
        assert!(parse_bhavcopy("<!DOCTYPE html><html><body>BSE</body></html>").is_err());
        assert!(parse_bhavcopy("TckrSymb,ISIN\nABB,INE117A01022\n").is_err());
        let only_debt = "TckrSymb,FinInstrmNm,ISIN,SctySrs\nGS2030,GOI LOAN,IN0020200001,G\n";
        assert!(parse_bhavcopy(only_debt).is_err());
        // A fixed-income group is dropped even with a company-style ISIN.
        let debenture = "TckrSymb,FinInstrmNm,ISIN,SctySrs\nXYZ28,XYZ NCD,INE000X07ABC,F\n";
        assert!(parse_bhavcopy(debenture).is_err());
    }

    #[test]
    fn candidate_dates_skip_weekends_and_count_back_from_today_in_india() {
        // Wednesday 2026-11-18 10:00 IST.
        let dates = candidate_dates(1_794_996_000_000);
        assert_eq!(dates[0], "20261118");
        assert_eq!(dates[1], "20261117");
        assert!(!dates.contains(&"20261114".to_owned()), "Saturday");
        assert!(!dates.contains(&"20261115".to_owned()), "Sunday");
        assert!(dates.contains(&"20261113".to_owned()), "Friday before");
        // 23:00 UTC on the 17th is already the 18th in India.
        let late = candidate_dates(1_794_996_000_000 - 11 * 3_600_000 + 1_000 * 3600);
        assert_eq!(late[0], "20261118");
    }

    #[tokio::test]
    async fn fetches_the_latest_file_walking_back_over_closed_days() {
        let server = MockServer::start().await;
        // Today's file isn't out yet (BSE answers with an HTML page), and the
        // day before is a normal 200.
        Mock::given(method("GET"))
            .and(path(
                "/download/BhavCopy/Equity/BhavCopy_BSE_CM_0_0_0_20261118_F_0000.CSV",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>BSE</html>"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(
                "/download/BhavCopy/Equity/BhavCopy_BSE_CM_0_0_0_20261117_F_0000.CSV",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(FIXTURE))
            .mount(&server)
            .await;
        let list = provider(&server).instruments(Exchange::Bse).await.unwrap();
        assert_eq!(list.len(), 5);
    }

    #[tokio::test]
    async fn a_block_stops_the_search_and_other_exchanges_are_unsupported() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403))
            .expect(1)
            .mount(&server)
            .await;
        let p = provider(&server);
        assert_eq!(
            p.instruments(Exchange::Bse).await.unwrap_err(),
            DataError::Blocked
        );
        assert_eq!(
            p.instruments(Exchange::Nse).await.unwrap_err(),
            DataError::Unsupported
        );
    }

    #[tokio::test]
    async fn gives_up_when_no_recent_day_has_a_file() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let err = provider(&server)
            .instruments(Exchange::Bse)
            .await
            .unwrap_err();
        assert_eq!(err, DataError::NotFound);
    }
}
