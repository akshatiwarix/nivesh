//! End-of-day activity (shares traded and traded value) from NSE's daily
//! bhavcopy, `sec_bhavdata_full_<DDMMYYYY>.csv` on the archive host.
//!
//! No free, no-login source reports per-stock volume for the whole market
//! during the day, so the *most active* movers list — and the heatmap's tile
//! sizes — are ranked by the last full session's traded value instead, and say
//! so. This is the same "dated bundled copy, refreshed in the background"
//! pattern as the index membership lists: a copy ships with the app, the
//! newest good download is kept in the store, and a failed download changes
//! nothing.
//!
//! Regenerate the bundled copy with
//! `cargo run -p nivesh-data --example refresh_seeds -- activity`.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use nivesh_core::Quote;
use nivesh_store::Store;
use serde::{Deserialize, Serialize};

use crate::DataError;
use crate::Mover;
use crate::constituents::ist_date;
use crate::http::HttpClient;
use crate::instruments::split_csv_line;

/// Who publishes the file, as shown next to the as-of date.
pub const SOURCE: &str = "NSE bhavcopy";

/// Where the file lives. The archive host serves it without a login; the
/// website host does not carry products at all.
pub const DEFAULT_BASES: [&str; 1] = ["https://archives.nseindia.com"];

/// A day's file appears after the close; a copy is refreshed daily.
pub const REFRESH_AFTER_MS: i64 = 20 * 60 * 60 * 1000;

/// How many days back a missing file is looked for: weekends and holidays.
const MAX_LOOKBACK_DAYS: u32 = 7;

const SET_KEY: &str = "activity.set";
const REFRESHED_KEY: &str = "activity.refreshed_at";

/// One stock's last full session.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ActivityRow {
    /// Shares traded.
    pub volume: u64,
    /// Traded value in rupees (the file reports lakhs).
    pub turnover: f64,
}

/// One session's activity, keyed by NSE symbol.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ActivityTable {
    /// `YYYY-MM-DD` of the session the rows belong to.
    pub as_of: String,
    pub source: String,
    pub rows: BTreeMap<String, ActivityRow>,
}

impl ActivityTable {
    /// A copy is worth keeping only when it has a real date and most of the
    /// market in it.
    fn plausible(&self) -> bool {
        self.as_of.len() == 10 && self.rows.len() >= 500
    }
}

/// Parses one `sec_bhavdata_full` CSV. The file pads every cell with a space,
/// and untraded stocks put `-` in the numeric columns; both are handled, and
/// rows without usable volume and turnover are dropped.
pub fn parse_activity_csv(text: &str) -> Result<ActivityTable, DataError> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header = lines
        .next()
        .ok_or_else(|| DataError::Parse("activity file is empty".into()))?;
    let names = ["SYMBOL", "DATE1", "TTL_TRD_QNTY", "TURNOVER_LACS"];
    let cols: Vec<usize> = names
        .iter()
        .map(|name| {
            split_csv_line(header)
                .iter()
                .position(|h| h.trim().eq_ignore_ascii_case(name))
                .ok_or_else(|| DataError::Parse(format!("activity file has no {name} column")))
        })
        .collect::<Result<_, _>>()?;
    let [symbol_col, date_col, volume_col, turnover_col] = [cols[0], cols[1], cols[2], cols[3]];

    let mut rows = BTreeMap::new();
    let mut as_of = String::new();
    for line in lines {
        let fields = split_csv_line(line);
        let cell = |col: usize| fields.get(col).map(|s| s.trim()).unwrap_or_default();
        let symbol = cell(symbol_col);
        if symbol.is_empty() {
            continue;
        }
        let volume = integer(cell(volume_col));
        let turnover = number(cell(turnover_col));
        let (Some(volume), Some(turnover_lacs)) = (volume, turnover) else {
            continue;
        };
        if !turnover_lacs.is_finite() || turnover_lacs <= 0.0 {
            continue;
        }
        if as_of.is_empty() {
            as_of = parse_file_date(cell(date_col))?;
        }
        rows.insert(
            symbol.to_ascii_uppercase(),
            ActivityRow {
                volume,
                turnover: turnover_lacs * 100_000.0,
            },
        );
    }
    if as_of.is_empty() {
        return Err(DataError::Parse("activity file has no rows".into()));
    }
    Ok(ActivityTable {
        as_of,
        source: SOURCE.to_owned(),
        rows,
    })
}

/// `" 1,234"` and `"1234"` both parse; `"-"` and junk do not.
fn number(cell: &str) -> Option<f64> {
    let cleaned = cell.replace(',', "");
    if cleaned.is_empty() || cleaned == "-" {
        return None;
    }
    cleaned.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// Whole counts of shares, as `TTL_TRD_QNTY` reports them.
fn integer(cell: &str) -> Option<u64> {
    let cleaned = cell.replace(',', "");
    if cleaned.is_empty() || cleaned == "-" {
        return None;
    }
    cleaned.parse::<u64>().ok()
}

/// The file's `DATE1` cell, `18-Nov-2026`, as `YYYY-MM-DD`.
fn parse_file_date(cell: &str) -> Result<String, DataError> {
    chrono::NaiveDate::parse_from_str(cell.trim(), "%d-%b-%Y")
        .map(|d| d.to_string())
        .map_err(|_| DataError::Parse(format!("activity file has a bad date {cell:?}")))
}

/// Downloads the file for one date from the first base that serves it.
async fn fetch_on(
    http: &HttpClient,
    bases: &[String],
    iso_date: &str,
) -> Result<ActivityTable, DataError> {
    let file_date = chrono::NaiveDate::parse_from_str(iso_date, "%Y-%m-%d")
        .map_err(|_| DataError::Parse(format!("bad activity date {iso_date:?}")))?
        .format("%d%m%Y")
        .to_string();
    let mut last_err = DataError::Unsupported;
    for base in bases {
        let url = format!(
            "{}/products/content/sec_bhavdata_full_{file_date}.csv",
            base.trim_end_matches('/')
        );
        match http.get(&url, &[("Accept", "text/csv,*/*")]).await {
            Ok(response) => match parse_activity_csv(&response.text()) {
                Ok(table) => return Ok(table),
                Err(e) => last_err = e,
            },
            Err(e) => last_err = e,
        }
    }
    Err(last_err)
}

/// The newest file at or before India's today, walking back over weekends and
/// holidays. The table's own `as_of` (from the file) is the truth, not the
/// date that was asked for.
pub async fn fetch_latest(
    http: &HttpClient,
    bases: &[String],
    now_ms: i64,
) -> Result<ActivityTable, DataError> {
    let mut last_err = DataError::Unsupported;
    let start = chrono::NaiveDate::parse_from_str(&ist_date(now_ms), "%Y-%m-%d")
        .map_err(|_| DataError::Parse("bad clock date".into()))?;
    for back in 0..=MAX_LOOKBACK_DAYS {
        let date = (start - chrono::Duration::days(i64::from(back))).to_string();
        match fetch_on(http, bases, &date).await {
            Ok(table) => return Ok(table),
            Err(e) => last_err = e,
        }
    }
    Err(last_err)
}

/// The `limit` constituents ranked by their last full session's traded value.
/// Constituents the file doesn't cover are not ranked. Ties keep the
/// constituents' order.
pub fn most_active_from_activity(
    table: &ActivityTable,
    constituents: &[Quote],
    limit: usize,
) -> Vec<Mover> {
    let mut ranked: Vec<(f64, &Quote)> = constituents
        .iter()
        .filter_map(|q| {
            let row = table.rows.get(q.id.symbol())?;
            (row.turnover.is_finite() && row.turnover > 0.0).then_some((row.turnover, q))
        })
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
    ranked.truncate(limit);
    ranked
        .into_iter()
        .map(|(_, q)| Mover { quote: q.clone() })
        .collect()
}

/// The activity table in use right now: the newest of the bundled copy and the
/// last good download kept in the store.
pub struct ActivityList {
    store: Option<Arc<Store>>,
    current: RwLock<Arc<ActivityTable>>,
}

impl ActivityList {
    /// Only the bundled copy, with nothing persisted (tests).
    pub fn bundled() -> Result<Self, DataError> {
        Ok(Self {
            store: None,
            current: RwLock::new(Arc::new(bundled()?)),
        })
    }

    /// A table of the caller's choosing, with nothing persisted (tests).
    #[cfg(test)]
    pub(crate) fn from_table(table: ActivityTable) -> Self {
        Self {
            store: None,
            current: RwLock::new(Arc::new(table)),
        }
    }

    /// The bundled copy, or a newer download kept in `store`.
    pub fn open(store: Arc<Store>) -> Result<Self, DataError> {
        let mut best = bundled()?;
        match store.setting(SET_KEY) {
            Ok(Some(json)) => match serde_json::from_str::<ActivityTable>(&json) {
                Ok(saved) if saved.as_of > best.as_of && saved.plausible() => best = saved,
                Ok(_) => {}
                Err(err) => tracing::warn!(%err, "ignoring unreadable stored activity table"),
            },
            Ok(None) => {}
            Err(err) => tracing::warn!(%err, "could not read stored activity table"),
        }
        Ok(Self {
            store: Some(store),
            current: RwLock::new(Arc::new(best)),
        })
    }

    pub fn snapshot(&self) -> Arc<ActivityTable> {
        match self.current.read() {
            Ok(guard) => Arc::clone(&guard),
            Err(poisoned) => Arc::clone(&poisoned.into_inner()),
        }
    }

    /// Adopts `table` if it is plausible and newer than the current one, and
    /// keeps it in the store. Returns whether it was adopted.
    pub fn adopt(&self, table: ActivityTable) -> bool {
        if !table.plausible() || table.as_of <= self.snapshot().as_of {
            return false;
        }
        if let (Some(store), Ok(json)) = (&self.store, serde_json::to_string(&table))
            && let Err(err) = store.set_setting(SET_KEY, &json)
        {
            tracing::warn!(%err, "could not store activity table");
        }
        match self.current.write() {
            Ok(mut guard) => *guard = Arc::new(table),
            Err(poisoned) => *poisoned.into_inner() = Arc::new(table),
        }
        true
    }

    /// Downloads a newer file when the last attempt that succeeded is a day
    /// old. A failed download changes nothing and is retried at the next
    /// check. Returns whether the table changed.
    pub async fn refresh_if_due(&self, http: &HttpClient, bases: &[String], now_ms: i64) -> bool {
        let last = self.store.as_ref().and_then(|s| {
            s.setting(REFRESHED_KEY)
                .ok()
                .flatten()
                .and_then(|v| v.parse::<i64>().ok())
        });
        if last.is_some_and(|t| now_ms - t < REFRESH_AFTER_MS) {
            return false;
        }
        match fetch_latest(http, bases, now_ms).await {
            Ok(table) => {
                if let Some(store) = &self.store
                    && let Err(err) = store.set_setting(REFRESHED_KEY, &now_ms.to_string())
                {
                    tracing::warn!(%err, "could not stamp the activity refresh");
                }
                let adopted = self.adopt(table);
                tracing::info!(adopted, "activity table refreshed");
                adopted
            }
            Err(err) => {
                tracing::warn!(%err, "activity refresh failed; keeping the current table");
                false
            }
        }
    }
}

/// The copy shipped with the app.
fn bundled() -> Result<ActivityTable, DataError> {
    const BUNDLED: &str = include_str!("../data/nse_activity.json");
    serde_json::from_str(BUNDLED).map_err(|e| DataError::Parse(format!("bundled activity: {e}")))
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::http::{HostLimits, HttpConfig, Limit};
    use nivesh_core::{Exchange, InstrumentId};

    /// A real recording, trimmed to its first 40 rows.
    const BHAV: &str = include_str!("../tests/fixtures/nse/sec_bhavdata_full_18112026.csv");

    fn http() -> HttpClient {
        let limits = Arc::new(HostLimits::new(Limit::new(
            std::time::Duration::from_millis(1),
            100,
        )));
        HttpClient::new(
            HttpConfig {
                max_attempts: 1,
                ..HttpConfig::default()
            },
            limits,
        )
        .unwrap()
    }

    fn quote(symbol: &str) -> Quote {
        Quote {
            id: InstrumentId::new(Exchange::Nse, symbol).unwrap(),
            name: None,
            ltp: 100.0,
            prev_close: 100.0,
            open: None,
            high: None,
            low: None,
            volume: None,
            week52_high: None,
            week52_low: None,
            upper_band: None,
            lower_band: None,
        }
    }

    #[test]
    fn parses_the_real_file_with_its_padded_cells() {
        let table = parse_activity_csv(BHAV).unwrap();
        assert_eq!(table.as_of, "2026-11-18");
        assert_eq!(table.source, SOURCE);
        let row = &table.rows["20MICRONS"];
        assert!(row.volume > 0);
        assert!(row.turnover > 0.0);
    }

    #[test]
    fn drops_rows_without_volume_and_rejects_junk() {
        let text = "SYMBOL, SERIES, DATE1, TTL_TRD_QNTY, TURNOVER_LACS\n\
                    A, EQ, 18-Nov-2026, -, -\n\
                    B, EQ, 18-Nov-2026, 100, 5.5\n\
                    C, EQ, 18-Nov-2026, , 5\n";
        let table = parse_activity_csv(text).unwrap();
        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows["B"].volume, 100);
        assert_eq!(table.rows["B"].turnover, 550_000.0);
        assert!(parse_activity_csv("SYMBOL,OTHER\nA,EQ\n").is_err());
        assert!(parse_activity_csv("").is_err());
    }

    #[test]
    fn a_bad_date_in_the_file_is_a_parse_error() {
        let text = "SYMBOL, SERIES, DATE1, TTL_TRD_QNTY, TURNOVER_LACS\n\
                    B, EQ, not-a-date, 100, 5\n";
        assert!(parse_activity_csv(text).is_err());
    }

    #[test]
    fn ranks_by_traded_value_and_keeps_unknown_symbols_out() {
        let table = parse_activity_csv(
            "SYMBOL, SERIES, DATE1, TTL_TRD_QNTY, TURNOVER_LACS\n\
             BIG, EQ, 18-Nov-2026, 10, 500\n\
             MID, EQ, 18-Nov-2026, 10, 50\n\
             TINY, EQ, 18-Nov-2026, 10, 5\n",
        )
        .unwrap();
        let constituents = [quote("MID"), quote("ABSENT"), quote("TINY"), quote("BIG")];
        let m = most_active_from_activity(&table, &constituents, 2);
        let symbols: Vec<&str> = m.iter().map(|mv| mv.quote.id.symbol()).collect();
        assert_eq!(symbols, ["BIG", "MID"]);
    }

    #[tokio::test]
    async fn fetch_latest_walks_back_to_the_last_trading_day() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/products/content/sec_bhavdata_full_19112026.csv"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/products/content/sec_bhavdata_full_18112026.csv"))
            .respond_with(ResponseTemplate::new(200).set_body_string(BHAV))
            .mount(&server)
            .await;
        // 2026-11-19 10:00 IST: the 19th's file is not out yet.
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-19T04:30:00Z")
            .unwrap()
            .timestamp_millis();
        let table = fetch_latest(&http(), &[server.uri()], now).await.unwrap();
        assert_eq!(table.as_of, "2026-11-18");
    }

    #[tokio::test]
    async fn every_day_missing_is_an_error() {
        let dead = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&dead)
            .await;
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-19T04:30:00Z")
            .unwrap()
            .timestamp_millis();
        assert!(fetch_latest(&http(), &[dead.uri()], now).await.is_err());
    }

    #[test]
    fn the_bundled_seed_is_plausible_and_dated() {
        let table = bundled().unwrap();
        assert!(table.plausible(), "{} rows", table.rows.len());
        assert_eq!(table.as_of.len(), 10);
        assert!(table.rows.contains_key("RELIANCE"));
    }

    #[test]
    fn adopt_wants_newer_and_plausible_tables_and_persists_them() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let list = ActivityList::open(Arc::clone(&store)).unwrap();
        let bundled_date = list.snapshot().as_of.clone();

        let mut thin = bundled().unwrap();
        thin.rows.clear();
        thin.rows.insert(
            "ONE".into(),
            ActivityRow {
                volume: 1,
                turnover: 1.0,
            },
        );
        thin.as_of = "2999-01-01".into();
        assert!(!list.adopt(thin), "implausible: one row");

        let mut newer = bundled().unwrap();
        newer.as_of = "2999-01-01".into();
        assert!(list.adopt(newer));
        assert_eq!(list.snapshot().as_of, "2999-01-01");
        // A restart reads the download back instead of the bundled copy.
        let again = ActivityList::open(store).unwrap();
        assert_eq!(again.snapshot().as_of, "2999-01-01");
        assert!(
            !list.adopt(bundled().unwrap()),
            "older than the current one"
        );
        assert!(!adopting_same_date_changes_nothing(&list, &bundled_date));
    }

    fn adopting_same_date_changes_nothing(list: &ActivityList, date: &str) -> bool {
        let mut same = bundled().unwrap();
        same.as_of = date.to_owned();
        list.adopt(same)
    }

    #[tokio::test]
    async fn refresh_runs_when_due_stamps_on_success_and_not_on_failure() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let list = ActivityList::open(Arc::clone(&store)).unwrap();
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-19T04:30:00Z")
            .unwrap()
            .timestamp_millis();

        let dead = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&dead)
            .await;
        assert!(!list.refresh_if_due(&http(), &[dead.uri()], now).await);
        assert!(store.setting(REFRESHED_KEY).unwrap().is_none());

        // A full day's file: enough rows to be plausible, and a day newer
        // than the bundled copy.
        let mut csv = String::from("SYMBOL, SERIES, DATE1, TTL_TRD_QNTY, TURNOVER_LACS\n");
        for i in 0..600 {
            csv.push_str(&format!("S{i:04}, EQ, 19-Nov-2026, 1000, 10.5\n"));
        }
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/products/content/sec_bhavdata_full_19112026.csv"))
            .respond_with(ResponseTemplate::new(200).set_body_string(csv))
            .mount(&server)
            .await;
        // 2026-11-20 10:00 IST: the 20th's file is not out yet, the 19th's is.
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-20T04:30:00Z")
            .unwrap()
            .timestamp_millis();
        assert!(list.refresh_if_due(&http(), &[server.uri()], now).await);
        assert_eq!(list.snapshot().as_of, "2026-11-19");
        assert!(store.setting(REFRESHED_KEY).unwrap().is_some());

        // Within the day: no download at all.
        assert!(
            !list
                .refresh_if_due(&http(), &[server.uri()], now + REFRESH_AFTER_MS - 1)
                .await
        );
    }
}
