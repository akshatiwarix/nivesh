//! Index membership lists: a dated, bundled fallback for when NSE's live
//! constituents are unavailable, and a popularity ranking for symbol search.
//!
//! The lists are NSE Indices' public constituent files (`ind_nifty50list.csv`
//! and friends). A copy is bundled with the app; the app tries to refresh it in
//! the background and keeps the newest copy it has in the local store. Every
//! list carries the date it was taken, and the UI shows that date wherever the
//! list is used instead of NSE's own live one.
//!
//! Regenerate the bundled copy with
//! `cargo run -p nivesh-data --example refresh_seeds -- constituents`.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};

use nivesh_core::{ConstituentsInfo, Exchange, IndexId, InstrumentId};
use nivesh_store::Store;
use serde::{Deserialize, Serialize};

use crate::DataError;
use crate::http::HttpClient;
use crate::instruments::split_csv_line;

/// The bundled lists (see [`ConstituentSet`]).
const BUNDLED: &str = include_str!("../data/index_constituents.json");

/// Who publishes the lists, as shown next to the as-of date.
pub const SOURCE: &str = "NSE Indices (niftyindices.com)";

/// Where the files live. NSE Indices' own site first, NSE's archive host second:
/// both serve the same files, and either can be blocked from a given network.
pub const DEFAULT_BASES: [&str; 2] = [
    "https://www.niftyindices.com/IndexConstituent",
    "https://archives.nseindia.com/content/indices",
];

/// `(list code, file stem, fewest rows that look like a real list)`.
///
/// Codes are [`IndexId::code`]s, plus two lists kept only for search ranking.
/// SENSEX is a BSE index and has no NSE file, so it has no fallback list.
pub const LISTS: &[(&str, &str, usize)] = &[
    ("NIFTY50", "ind_nifty50list", 40),
    ("BANKNIFTY", "ind_niftybanklist", 5),
    ("NIFTYIT", "ind_niftyitlist", 5),
    ("NIFTYAUTO", "ind_niftyautolist", 5),
    ("NIFTYPHARMA", "ind_niftypharmalist", 5),
    ("NIFTYFMCG", "ind_niftyfmcglist", 5),
    ("NIFTYMETAL", "ind_niftymetallist", 5),
    ("NIFTYREALTY", "ind_niftyrealtylist", 5),
    ("NIFTYENERGY", "ind_niftyenergylist", 5),
    ("NIFTYPSUBANK", "ind_niftypsubanklist", 5),
    ("NIFTYFINSERVICE", "ind_niftyfinancelist", 10),
    ("NIFTYMEDIA", "ind_niftymedialist", 5),
    ("NIFTYNEXT50", "ind_niftynext50list", 40),
    ("NIFTY500", "ind_nifty500list", 400),
];

/// How long a downloaded copy is trusted before another download is attempted.
/// Index membership changes twice a year; a week keeps the copy current enough.
pub const REFRESH_AFTER_MS: i64 = 7 * 24 * 60 * 60 * 1000;

const SET_KEY: &str = "constituents.set";
const REFRESHED_KEY: &str = "constituents.refreshed_at";

/// Every list, taken on one date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstituentSet {
    /// `YYYY-MM-DD`.
    pub as_of: String,
    pub source: String,
    /// List code to NSE symbols, in the publisher's order.
    pub lists: BTreeMap<String, Vec<String>>,
}

impl ConstituentSet {
    /// The copy shipped with the app.
    pub fn bundled() -> Result<Self, DataError> {
        serde_json::from_str(BUNDLED).map_err(|e| DataError::Parse(format!("bundled lists: {e}")))
    }

    /// Search-ranking tier of a symbol: 0 for NIFTY 50, 1 for NIFTY NEXT 50,
    /// 2 for the rest of NIFTY 500. Symbols in no list are not in the map.
    pub fn popularity(&self) -> HashMap<String, u8> {
        let mut map = HashMap::new();
        // Best tier last, so it overwrites the weaker ones.
        for (code, tier) in [("NIFTY500", 2u8), ("NIFTYNEXT50", 1), ("NIFTY50", 0)] {
            for symbol in self.lists.get(code).into_iter().flatten() {
                map.insert(symbol.to_ascii_uppercase(), tier);
            }
        }
        map
    }

    fn plausible(&self) -> bool {
        LISTS.iter().all(|(code, _, min)| {
            self.lists
                .get(*code)
                .is_some_and(|symbols| symbols.len() >= *min)
        })
    }
}

/// Reads the symbols of one NSE Indices constituent CSV (`Symbol` column, by
/// header name). Rows with unusable symbols are skipped.
pub fn parse_constituent_csv(text: &str) -> Result<Vec<String>, DataError> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header = lines
        .next()
        .ok_or_else(|| DataError::Parse("constituent list is empty".into()))?;
    let col = split_csv_line(header)
        .iter()
        .position(|h| h.trim().eq_ignore_ascii_case("symbol"))
        .ok_or_else(|| DataError::Parse("constituent list has no Symbol column".into()))?;
    let mut out = Vec::new();
    for line in lines {
        let fields = split_csv_line(line);
        let Some(symbol) = fields.get(col).map(|s| s.trim()).filter(|s| !s.is_empty()) else {
            continue;
        };
        if InstrumentId::new(Exchange::Nse, symbol).is_ok() && !out.iter().any(|s| s == symbol) {
            out.push(symbol.to_owned());
        }
    }
    if out.is_empty() {
        return Err(DataError::Parse("constituent list has no rows".into()));
    }
    Ok(out)
}

/// Downloads every list and stamps the set with `as_of`. All or nothing: a set
/// with one list from a different day would make the single date a lie.
pub async fn fetch_set(
    http: &HttpClient,
    bases: &[String],
    as_of: &str,
) -> Result<ConstituentSet, DataError> {
    let mut lists = BTreeMap::new();
    for (code, stem, min) in LISTS {
        let mut last_err = DataError::Unsupported;
        let mut found = None;
        for base in bases {
            let url = format!("{}/{stem}.csv", base.trim_end_matches('/'));
            let attempt = match http.get(&url, &[("Accept", "text/csv,*/*")]).await {
                Ok(response) => parse_constituent_csv(&response.text()),
                Err(e) => Err(e),
            };
            match attempt {
                Ok(symbols) if symbols.len() >= *min => {
                    found = Some(symbols);
                    break;
                }
                Ok(symbols) => {
                    last_err = DataError::Parse(format!(
                        "{code}: list too short ({} rows)",
                        symbols.len()
                    ));
                }
                Err(e) => last_err = e,
            }
        }
        match found {
            Some(symbols) => {
                lists.insert((*code).to_owned(), symbols);
            }
            None => return Err(last_err),
        }
    }
    Ok(ConstituentSet {
        as_of: as_of.to_owned(),
        source: SOURCE.to_owned(),
        lists,
    })
}

/// The date in India, `YYYY-MM-DD`, for stamping a download.
pub fn ist_date(now_ms: i64) -> String {
    const IST_OFFSET_MS: i64 = 330 * 60 * 1000;
    chrono::DateTime::from_timestamp_millis(now_ms + IST_OFFSET_MS)
        .map(|t| t.date_naive().to_string())
        .unwrap_or_default()
}

/// The membership lists in use right now: the newest of the bundled copy and
/// the last good download kept in the store.
pub struct ConstituentsList {
    store: Option<Arc<Store>>,
    current: RwLock<Arc<ConstituentSet>>,
}

impl ConstituentsList {
    /// Only the bundled copy, with nothing persisted (tests).
    pub fn bundled() -> Result<Self, DataError> {
        Ok(Self {
            store: None,
            current: RwLock::new(Arc::new(ConstituentSet::bundled()?)),
        })
    }

    /// The bundled copy, or a newer download kept in `store`.
    pub fn open(store: Arc<Store>) -> Result<Self, DataError> {
        let mut best = ConstituentSet::bundled()?;
        match store.setting(SET_KEY) {
            Ok(Some(json)) => match serde_json::from_str::<ConstituentSet>(&json) {
                Ok(saved) if saved.as_of > best.as_of && saved.plausible() => best = saved,
                Ok(_) => {}
                Err(err) => tracing::warn!(%err, "ignoring unreadable stored constituent lists"),
            },
            Ok(None) => {}
            Err(err) => tracing::warn!(%err, "could not read stored constituent lists"),
        }
        Ok(Self {
            store: Some(store),
            current: RwLock::new(Arc::new(best)),
        })
    }

    pub fn snapshot(&self) -> Arc<ConstituentSet> {
        match self.current.read() {
            Ok(guard) => Arc::clone(&guard),
            Err(poisoned) => Arc::clone(&poisoned.into_inner()),
        }
    }

    /// The members of `index`, with the list's date and source. `None` when
    /// there is no list for it (SENSEX).
    pub fn members(&self, index: IndexId) -> Option<(Vec<InstrumentId>, ConstituentsInfo)> {
        let set = self.snapshot();
        let symbols = set.lists.get(index.code())?;
        let ids: Vec<InstrumentId> = symbols
            .iter()
            .filter_map(|s| InstrumentId::new(Exchange::Nse, s).ok())
            .collect();
        (!ids.is_empty()).then(|| {
            (
                ids,
                ConstituentsInfo {
                    as_of: set.as_of.clone(),
                    source: set.source.clone(),
                },
            )
        })
    }

    /// Search-ranking tiers (see [`ConstituentSet::popularity`]).
    pub fn popularity(&self) -> HashMap<String, u8> {
        self.snapshot().popularity()
    }

    /// Adopts `set` if it is plausible and newer than the current one, and keeps
    /// it in the store. Returns whether it was adopted.
    pub fn adopt(&self, set: ConstituentSet) -> bool {
        if !set.plausible() || set.as_of <= self.snapshot().as_of {
            return false;
        }
        if let (Some(store), Ok(json)) = (&self.store, serde_json::to_string(&set))
            && let Err(err) = store.set_setting(SET_KEY, &json)
        {
            tracing::warn!(%err, "could not store constituent lists");
        }
        match self.current.write() {
            Ok(mut guard) => *guard = Arc::new(set),
            Err(poisoned) => *poisoned.into_inner() = Arc::new(set),
        }
        true
    }

    /// Downloads fresh lists when the last attempt that succeeded is more than a
    /// week old. A failed download changes nothing and is retried at the next
    /// check. Returns whether the lists changed.
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
        match fetch_set(http, bases, &ist_date(now_ms)).await {
            Ok(set) => {
                if let Some(store) = &self.store
                    && let Err(err) = store.set_setting(REFRESHED_KEY, &now_ms.to_string())
                {
                    tracing::warn!(%err, "could not stamp the constituent refresh");
                }
                let adopted = self.adopt(set);
                tracing::info!(adopted, "constituent lists refreshed");
                adopted
            }
            Err(err) => {
                tracing::warn!(%err, "constituent refresh failed; keeping the current lists");
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::http::{HostLimits, HttpConfig, Limit};

    const NIFTY50_CSV: &str = include_str!("../tests/fixtures/nse/ind_nifty50list.csv");

    fn http() -> HttpClient {
        let limits = Arc::new(HostLimits::new(Limit::new(Duration::from_millis(1), 100)));
        let config = HttpConfig {
            max_attempts: 1,
            ..HttpConfig::default()
        };
        HttpClient::new(config, limits).unwrap()
    }

    /// A CSV in NSE Indices' format with `n` rows named `<prefix>1..n`.
    fn csv(prefix: &str, n: usize) -> String {
        let mut text = String::from("Company Name,Industry,Symbol,Series,ISIN Code\n");
        for i in 1..=n {
            text.push_str(&format!("Co {i} Ltd.,Misc,{prefix}{i},EQ,INE00000000{i}\n"));
        }
        text
    }

    fn set(as_of: &str) -> ConstituentSet {
        let lists = LISTS
            .iter()
            .map(|(code, _, min)| {
                let symbols = (0..*min).map(|i| format!("{code}{i}")).collect();
                ((*code).to_owned(), symbols)
            })
            .collect();
        ConstituentSet {
            as_of: as_of.to_owned(),
            source: SOURCE.to_owned(),
            lists,
        }
    }

    #[test]
    fn parses_the_real_nifty_50_file() {
        let symbols = parse_constituent_csv(NIFTY50_CSV).unwrap();
        assert_eq!(symbols.len(), 50);
        assert!(symbols.contains(&"RELIANCE".to_owned()));
        assert!(symbols.contains(&"M&M".to_owned()));
        assert!(symbols.contains(&"BAJAJ-AUTO".to_owned()));
    }

    #[test]
    fn csv_skips_bad_rows_and_duplicates_and_rejects_junk() {
        let text = "Company Name,Industry,Symbol\nA,x,AAA\nB,x,\nC,x,!!\nD,x,AAA\nE,x,BBB\n";
        assert_eq!(parse_constituent_csv(text).unwrap(), ["AAA", "BBB"]);
        assert!(parse_constituent_csv("").is_err());
        assert!(parse_constituent_csv("<html>Access Denied</html>").is_err());
        assert!(parse_constituent_csv("Company Name,Symbol\n").is_err());
    }

    #[test]
    fn the_bundled_lists_are_complete_and_dated() {
        let set = ConstituentSet::bundled().unwrap();
        assert!(set.plausible(), "every list meets its size floor");
        assert_eq!(set.as_of.len(), 10, "{}", set.as_of);
        assert_eq!(set.lists["NIFTY50"].len(), 50);
        assert!(set.lists["NIFTY50"].contains(&"RELIANCE".to_owned()));
        assert!(set.lists["BANKNIFTY"].contains(&"HDFCBANK".to_owned()));
    }

    #[test]
    fn every_nse_catalogue_index_has_a_bundled_list() {
        let set = ConstituentSet::bundled().unwrap();
        for index in IndexId::all() {
            let has_list = set.lists.contains_key(index.code());
            // SENSEX is a BSE index: NSE Indices publishes no file for it.
            assert_eq!(has_list, index.nse_name().is_some(), "{}", index.code());
        }
    }

    #[test]
    fn popularity_tiers_prefer_the_smaller_index() {
        let mut s = set("2026-01-01");
        s.lists.insert("NIFTY50".into(), vec!["AAA".into()]);
        s.lists
            .insert("NIFTYNEXT50".into(), vec!["BBB".into(), "AAA".into()]);
        s.lists
            .insert("NIFTY500".into(), vec!["CCC".into(), "BBB".into()]);
        let pop = s.popularity();
        assert_eq!(pop["AAA"], 0);
        assert_eq!(pop["BBB"], 1);
        assert_eq!(pop["CCC"], 2);
        assert!(!pop.contains_key("ZZZ"));
    }

    #[test]
    fn members_carry_the_date_and_source() {
        let list = ConstituentsList::bundled().unwrap();
        let (ids, info) = list.members(IndexId::NiftyBank).unwrap();
        assert!(ids.iter().any(|i| i.symbol() == "HDFCBANK"));
        assert!(ids.iter().all(|i| i.exchange() == Exchange::Nse));
        assert_eq!(info.as_of, ConstituentSet::bundled().unwrap().as_of);
        assert_eq!(info.source, SOURCE);
        assert!(list.members(IndexId::Sensex).is_none());
    }

    #[test]
    fn adopt_wants_newer_and_complete_sets_and_persists_them() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let list = ConstituentsList::open(Arc::clone(&store)).unwrap();
        let bundled_date = list.snapshot().as_of.clone();

        assert!(
            !list.adopt(set("2000-01-01")),
            "older than the bundled copy"
        );
        assert!(!list.adopt(set(&bundled_date)), "same date");
        let mut thin = set("2999-01-01");
        thin.lists.insert("NIFTY50".into(), vec!["ONLY".into()]);
        assert!(!list.adopt(thin), "implausible: NIFTY 50 with one name");
        assert_eq!(list.snapshot().as_of, bundled_date);

        assert!(list.adopt(set("2999-01-01")));
        assert_eq!(list.snapshot().as_of, "2999-01-01");
        // A restart reads the download back instead of the bundled copy.
        let again = ConstituentsList::open(store).unwrap();
        assert_eq!(again.snapshot().as_of, "2999-01-01");
    }

    #[test]
    fn an_unreadable_stored_copy_falls_back_to_the_bundled_one() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        store.set_setting(SET_KEY, "{not json").unwrap();
        let list = ConstituentsList::open(store).unwrap();
        assert_eq!(
            list.snapshot().as_of,
            ConstituentSet::bundled().unwrap().as_of
        );
    }

    #[test]
    fn ist_dates_roll_over_at_half_past_six_utc() {
        // 18:29:59 UTC is 23:59:59 IST, still the 17th; a second later it is
        // midnight in India.
        let t = chrono::DateTime::parse_from_rfc3339("2026-11-17T18:29:59Z")
            .unwrap()
            .timestamp_millis();
        assert_eq!(ist_date(t), "2026-11-17");
        assert_eq!(ist_date(t + 1000), "2026-11-18");
    }

    async fn serve_all(server: &MockServer) {
        for (code, stem, min) in LISTS {
            let body = if *code == "NIFTY50" {
                NIFTY50_CSV.to_owned()
            } else {
                csv(code, *min)
            };
            Mock::given(method("GET"))
                .and(path(format!("/{stem}.csv")))
                .respond_with(ResponseTemplate::new(200).set_body_string(body))
                .mount(server)
                .await;
        }
    }

    #[tokio::test]
    async fn fetch_set_downloads_every_list_and_stamps_the_date() {
        let server = MockServer::start().await;
        serve_all(&server).await;
        let set = fetch_set(&http(), &[server.uri()], "2026-11-18")
            .await
            .unwrap();
        assert_eq!(set.as_of, "2026-11-18");
        assert_eq!(set.lists.len(), LISTS.len());
        assert_eq!(set.lists["NIFTY50"].len(), 50);
        assert!(set.plausible());
    }

    #[tokio::test]
    async fn fetch_set_uses_the_second_host_when_the_first_is_blocked() {
        let blocked = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403).set_body_string("Access Denied"))
            .mount(&blocked)
            .await;
        let html = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>Hi</html>"))
            .mount(&html)
            .await;
        let good = MockServer::start().await;
        serve_all(&good).await;
        let bases = [blocked.uri(), html.uri(), good.uri()];
        let set = fetch_set(&http(), &bases, "2026-11-18").await.unwrap();
        assert_eq!(set.lists["NIFTY50"].len(), 50);
    }

    #[tokio::test]
    async fn fetch_set_is_all_or_nothing() {
        let server = MockServer::start().await;
        serve_all(&server).await;
        // Replace one list with a 2-row file: the whole download is rejected.
        server.reset().await;
        for (code, stem, min) in LISTS {
            let rows = if *code == "NIFTYIT" { 2 } else { *min };
            Mock::given(method("GET"))
                .and(path(format!("/{stem}.csv")))
                .respond_with(ResponseTemplate::new(200).set_body_string(csv(code, rows)))
                .mount(&server)
                .await;
        }
        let err = fetch_set(&http(), &[server.uri()], "2026-11-18")
            .await
            .unwrap_err();
        assert!(matches!(err, DataError::Parse(_)), "{err:?}");
    }

    #[tokio::test]
    async fn refresh_runs_when_due_stamps_on_success_and_not_on_failure() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let list = ConstituentsList::open(Arc::clone(&store)).unwrap();
        let now = 1_900_000_000_000; // 2030-03-17, well after the bundled date

        // A blocked network: nothing changes, nothing is stamped, retry later.
        let dead = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&dead)
            .await;
        assert!(!list.refresh_if_due(&http(), &[dead.uri()], now).await);
        assert!(store.setting(REFRESHED_KEY).unwrap().is_none());

        let server = MockServer::start().await;
        serve_all(&server).await;
        let bases = [server.uri()];
        assert!(list.refresh_if_due(&http(), &bases, now).await);
        assert_eq!(list.snapshot().as_of, ist_date(now));
        assert!(store.setting(REFRESHED_KEY).unwrap().is_some());

        // Within the week: no download at all.
        server.reset().await;
        assert!(
            !list
                .refresh_if_due(&http(), &bases, now + REFRESH_AFTER_MS - 1)
                .await
        );
    }
}
