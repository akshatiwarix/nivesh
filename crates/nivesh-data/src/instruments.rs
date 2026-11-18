//! The local instrument master and symbol search.
//!
//! Listed equities are refreshed daily from the exchange's list into SQLite,
//! with a bundled seed for the first offline run. Search runs against an
//! in-memory index, so it never waits on the network or the disk: typing in
//! the command bar stays well inside the 16 ms keystroke budget.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use nivesh_core::{Exchange, Instrument, InstrumentId};
use nivesh_store::{InstrumentRow, Store};

use crate::constituents::ConstituentSet;
use crate::{DataError, DataProvider};

/// NSE's equity list as shipped with the app (symbol, name, series, ISIN).
const SEED_NSE: &str = include_str!("../data/nse_equity_seed.csv");

/// BSE's equity list as shipped with the app (same columns; the symbol is BSE's
/// ticker and the series is BSE's group, `A`, `B`, `T`...).
const SEED_BSE: &str = include_str!("../data/bse_equity_seed.csv");

/// A refresh is attempted when the stored list is older than this.
pub const REFRESH_AFTER_MS: i64 = 24 * 60 * 60 * 1000;

/// A list shorter than this is treated as a broken download, not a delisting
/// spree, and never replaces the stored one.
pub const MIN_PLAUSIBLE_ROWS: usize = 500;

// ---- CSV ------------------------------------------------------------------

/// Splits one CSV line, honouring double quotes (`"A, B"`, `""` escapes).
pub(crate) fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => fields.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    fields.push(cur);
    fields
}

/// Parses NSE's `EQUITY_L.csv`, or the bundled seed (same columns, fewer).
/// Columns are found by header name, so extra columns and NSE's stray spaces in
/// header names are fine. Rows with unusable symbols are skipped.
pub fn parse_equity_csv(text: &str) -> Result<Vec<Instrument>, DataError> {
    parse_list_csv(text, Exchange::Nse)
}

/// [`parse_equity_csv`] for any exchange's list in the same columns.
pub fn parse_list_csv(text: &str, exchange: Exchange) -> Result<Vec<Instrument>, DataError> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header = lines
        .next()
        .ok_or_else(|| DataError::Parse("instrument list is empty".into()))?;
    let names: Vec<String> = split_csv_line(header)
        .iter()
        .map(|h| h.trim().to_ascii_uppercase())
        .collect();
    let col = |wanted: &str| names.iter().position(|n| n == wanted);
    let (Some(sym_col), Some(name_col)) = (col("SYMBOL"), col("NAME OF COMPANY")) else {
        return Err(DataError::Parse(
            "instrument list has no SYMBOL / NAME OF COMPANY columns".into(),
        ));
    };
    let (series_col, isin_col) = (col("SERIES"), col("ISIN NUMBER"));

    let mut out = Vec::new();
    for line in lines {
        let f = split_csv_line(line);
        let get = |i: usize| f.get(i).map(|v| v.trim()).filter(|v| !v.is_empty());
        let (Some(symbol), Some(name)) = (get(sym_col), get(name_col)) else {
            continue;
        };
        let Ok(id) = InstrumentId::new(exchange, symbol) else {
            continue;
        };
        out.push(Instrument {
            id,
            name: name.to_owned(),
            isin: isin_col.and_then(get).map(str::to_owned),
            series: series_col.and_then(get).map(str::to_owned),
        });
    }
    if out.is_empty() {
        return Err(DataError::Parse(
            "instrument list has no usable rows".into(),
        ));
    }
    Ok(out)
}

/// Writes a list in the seed format [`parse_list_csv`] reads.
pub fn write_list_csv(list: &[Instrument]) -> String {
    fn quote(field: &str) -> String {
        if field.contains([',', '"']) {
            format!("\"{}\"", field.replace('"', "\"\""))
        } else {
            field.to_owned()
        }
    }
    let mut out = String::from("SYMBOL,NAME OF COMPANY,SERIES,ISIN NUMBER\n");
    for i in list {
        out.push_str(&format!(
            "{},{},{},{}\n",
            quote(i.id.symbol()),
            quote(&i.name),
            quote(i.series.as_deref().unwrap_or("")),
            quote(i.isin.as_deref().unwrap_or(""))
        ));
    }
    out
}

// ---- search ---------------------------------------------------------------

struct Entry {
    instrument: Instrument,
    /// Upper-case symbol and name, precomputed.
    sym: String,
    name: String,
    /// The name without spaces, for fuzzy matching ("tatamot" ~ "TATA MOTORS").
    name_packed: String,
    words: Vec<String>,
    /// How well known the stock is: 0 = NIFTY 50, 1 = NIFTY NEXT 50,
    /// 2 = rest of NIFTY 500, [`UNRANKED`] = none of them. Lower sorts first.
    popularity: u32,
    /// A BSE listing of a company that is also listed on NSE (same ISIN).
    /// Kept for lookups by id, hidden from search: one result per company,
    /// the NSE one.
    shadowed: bool,
}

/// The popularity of a stock in no bundled index.
const UNRANKED: u32 = 3;

/// An immutable, ranked in-memory index. Rebuilt (not mutated) on refresh.
pub struct InstrumentIndex {
    entries: Vec<Entry>,
}

/// Lower is better. Compared lexicographically: match tier first, then the
/// tier's own keys (popularity breaks ties between matches of equal quality).
type Rank = (u8, u32, u32, usize);

impl InstrumentIndex {
    pub fn new(instruments: Vec<Instrument>) -> Self {
        let entries = instruments
            .into_iter()
            .map(|instrument| {
                let name = instrument.name.to_ascii_uppercase();
                Entry {
                    sym: instrument.id.symbol().to_ascii_uppercase(),
                    name_packed: name.chars().filter(|c| c.is_alphanumeric()).collect(),
                    words: name
                        .split(|c: char| !c.is_alphanumeric())
                        .filter(|w| !w.is_empty())
                        .map(str::to_owned)
                        .collect(),
                    name,
                    instrument,
                    popularity: UNRANKED,
                    shadowed: false,
                }
            })
            .collect::<Vec<_>>();
        let mut index = Self { entries };
        index.shadow_dual_listings();
        index
    }

    /// Marks BSE rows whose ISIN an NSE row also has. Rows without an ISIN
    /// can't be matched and stay visible.
    fn shadow_dual_listings(&mut self) {
        let on_nse: std::collections::HashSet<&str> = self
            .entries
            .iter()
            .filter(|e| e.instrument.id.exchange() == Exchange::Nse)
            .filter_map(|e| e.instrument.isin.as_deref())
            .collect();
        let shadowed: Vec<bool> = self
            .entries
            .iter()
            .map(|e| {
                e.instrument.id.exchange() == Exchange::Bse
                    && e.instrument
                        .isin
                        .as_deref()
                        .is_some_and(|isin| on_nse.contains(isin))
            })
            .collect();
        for (e, s) in self.entries.iter_mut().zip(shadowed) {
            e.shadowed = s;
        }
    }

    /// Ranks well-known stocks first among equally good matches. `tiers` maps
    /// upper-case symbols to 0 (NIFTY 50), 1 (NIFTY NEXT 50) or 2 (NIFTY 500).
    /// Only NSE symbols are ranked: a BSE-only ticker that happens to spell a
    /// NIFTY symbol is a different company.
    pub fn with_popularity(mut self, tiers: &HashMap<String, u8>) -> Self {
        for e in &mut self.entries {
            if e.instrument.id.exchange() == Exchange::Nse {
                e.popularity = tiers
                    .get(&e.sym)
                    .map_or(UNRANKED, |t| u32::from(*t).min(UNRANKED));
            }
        }
        self
    }

    /// `(symbol, company name, popularity tier)` of every NSE listing, for
    /// tagging headlines (see [`crate::news::TagIndex`]). Lower tier is better:
    /// 0 NIFTY 50, 1 NIFTY NEXT 50, 2 NIFTY 500, 3 the rest.
    pub fn tag_entries(&self) -> Vec<(String, String, u32)> {
        self.entries
            .iter()
            .filter(|e| e.instrument.id.exchange() == Exchange::Nse)
            .map(|e| (e.sym.clone(), e.instrument.name.clone(), e.popularity))
            .collect()
    }

    /// Every listing, including the BSE ones that search hides.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, id: &InstrumentId) -> Option<&Instrument> {
        self.entries
            .iter()
            .find(|e| &e.instrument.id == id)
            .map(|e| &e.instrument)
    }

    /// The best `limit` matches, best first: exact symbol, then symbol prefix
    /// (well-known stocks first, then shorter symbols), then name word-prefix,
    /// then substring, then fuzzy (subsequence) matches on symbol and name.
    /// Popularity only ever breaks ties inside a tier: a popular name match
    /// never outranks a symbol match.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Instrument> {
        let q = query.trim().to_ascii_uppercase();
        let packed: String = q.chars().filter(|c| !c.is_whitespace()).collect();
        if packed.is_empty() || limit == 0 {
            return Vec::new();
        }
        let tokens: Vec<&str> = q.split_whitespace().collect();
        let mut hits: Vec<(Rank, &Entry)> = self
            .entries
            .iter()
            .filter(|e| !e.shadowed)
            .filter_map(|e| rank(e, &q, &packed, &tokens).map(|r| (r, e)))
            .collect();
        hits.sort_by(|(ra, a), (rb, b)| {
            ra.cmp(rb)
                .then_with(|| exchange_order(a).cmp(&exchange_order(b)))
                .then_with(|| a.sym.len().cmp(&b.sym.len()))
                .then_with(|| a.sym.cmp(&b.sym))
        });
        hits.into_iter()
            .take(limit)
            .map(|(_, e)| e.instrument.clone())
            .collect()
    }
}

fn exchange_order(e: &Entry) -> u8 {
    match e.instrument.id.exchange() {
        Exchange::Nse => 0,
        Exchange::Bse => 1,
    }
}

fn rank(e: &Entry, q: &str, packed: &str, tokens: &[&str]) -> Option<Rank> {
    // Tier 0: the symbol itself.
    if e.sym == packed {
        return Some((0, 0, 0, 0));
    }
    // Tier 1: symbol prefix. Well-known stocks first ("relia" is RELIANCE, not
    // RELIABLE), then shorter symbols (closer to exact).
    if e.sym.starts_with(packed) {
        return Some((1, e.popularity, e.sym.len() as u32, 0));
    }
    // Tier 2: every query word starts some word of the name.
    if tokens
        .iter()
        .all(|t| e.words.iter().any(|w| w.starts_with(t)))
    {
        let starts = u32::from(!e.name.starts_with(q));
        return Some((2, e.popularity, starts, e.name.len()));
    }
    // One- and two-letter queries only match prefixes: anything looser is noise.
    if packed.len() < 3 {
        return None;
    }
    if e.sym.contains(packed) {
        return Some((3, e.popularity, 0, e.sym.len()));
    }
    if e.name.contains(q) {
        return Some((4, e.popularity, 0, e.name.len()));
    }
    // Fuzzy matches: a tighter match beats a popular loose one.
    if let Some(gaps) = subsequence_gaps(&e.sym, packed) {
        return Some((5, gaps, e.popularity, e.sym.len()));
    }
    subsequence_gaps(&e.name_packed, packed).map(|gaps| (6, gaps, e.popularity, e.name.len()))
}

/// If `needle` is a subsequence of `haystack`, the number of skipped
/// characters between the first and last matched ones (tighter is better).
fn subsequence_gaps(haystack: &str, needle: &str) -> Option<u32> {
    let mut want = needle.chars();
    let mut next = want.next()?;
    let mut first = None;
    for (i, c) in haystack.chars().enumerate() {
        if c != next {
            continue;
        }
        let start = *first.get_or_insert(i);
        match want.next() {
            Some(n) => next = n,
            None => {
                let span = i - start + 1;
                return Some((span - needle.chars().count()) as u32);
            }
        }
    }
    None
}

// ---- master ---------------------------------------------------------------

fn refreshed_key(exchange: Exchange) -> String {
    format!("instruments.refreshed_at.{}", exchange.code())
}

fn store_err(err: nivesh_store::StoreError) -> DataError {
    DataError::Upstream(format!("store: {err}"))
}

fn to_row(i: &Instrument) -> InstrumentRow {
    InstrumentRow {
        exchange: i.id.exchange().code().to_owned(),
        symbol: i.id.symbol().to_owned(),
        name: i.name.clone(),
        isin: i.isin.clone(),
        series: i.series.clone(),
    }
}

fn from_row(row: InstrumentRow) -> Option<Instrument> {
    let exchange: Exchange = row.exchange.parse().ok()?;
    Some(Instrument {
        id: InstrumentId::new(exchange, &row.symbol).ok()?,
        name: row.name,
        isin: row.isin,
        series: row.series,
    })
}

/// The instrument master: SQLite-backed, searched through an in-memory index.
pub struct InstrumentMaster {
    store: Arc<Store>,
    index: RwLock<Arc<InstrumentIndex>>,
    min_rows: usize,
    popularity: RwLock<Arc<HashMap<String, u8>>>,
}

impl InstrumentMaster {
    /// Loads the stored lists, seeding each exchange from its bundled list when
    /// the store has none (first run, possibly offline).
    pub fn open(store: Arc<Store>) -> Result<Self, DataError> {
        let tiers = ConstituentSet::bundled()?.popularity();
        Self::open_ranked(store, tiers)
    }

    /// Like [`open`](Self::open), ranking search results by `tiers` (see
    /// [`InstrumentIndex::with_popularity`]).
    pub fn open_ranked(store: Arc<Store>, tiers: HashMap<String, u8>) -> Result<Self, DataError> {
        let mut stored = store.instruments().map_err(store_err)?;
        let mut seeded = false;
        for (exchange, seed) in [(Exchange::Nse, SEED_NSE), (Exchange::Bse, SEED_BSE)] {
            if !stored.iter().any(|r| r.exchange == exchange.code()) {
                let list = parse_list_csv(seed, exchange)?;
                let rows: Vec<_> = list.iter().map(to_row).collect();
                store
                    .replace_instruments(exchange.code(), &rows)
                    .map_err(store_err)?;
                seeded = true;
            }
        }
        if seeded {
            stored = store.instruments().map_err(store_err)?;
        }
        let instruments = stored.into_iter().filter_map(from_row).collect();
        Ok(Self {
            store,
            index: RwLock::new(Arc::new(
                InstrumentIndex::new(instruments).with_popularity(&tiers),
            )),
            min_rows: MIN_PLAUSIBLE_ROWS,
            popularity: RwLock::new(Arc::new(tiers)),
        })
    }

    /// Swaps in new popularity tiers (after the membership lists are
    /// refreshed) and re-ranks.
    pub fn set_popularity(&self, tiers: HashMap<String, u8>) -> Result<(), DataError> {
        match self.popularity.write() {
            Ok(mut guard) => *guard = Arc::new(tiers),
            Err(poisoned) => *poisoned.into_inner() = Arc::new(tiers),
        }
        self.rebuild()
    }

    /// For tests with small lists.
    pub fn with_min_rows(mut self, min_rows: usize) -> Self {
        self.min_rows = min_rows;
        self
    }

    fn snapshot(&self) -> Arc<InstrumentIndex> {
        match self.index.read() {
            Ok(guard) => Arc::clone(&guard),
            Err(poisoned) => Arc::clone(&poisoned.into_inner()),
        }
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<Instrument> {
        self.snapshot().search(query, limit)
    }

    pub fn get(&self, id: &InstrumentId) -> Option<Instrument> {
        self.snapshot().get(id).cloned()
    }

    /// A headline tagger over the current NSE listings.
    pub fn tagger(&self) -> crate::news::TagIndex {
        crate::news::TagIndex::new(&self.snapshot().tag_entries())
    }

    pub fn len(&self) -> usize {
        self.snapshot().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Refreshes every exchange whose list is older than a day, from the first
    /// provider that can supply it. A failed or implausible download keeps the
    /// current list. Returns the exchanges that were updated.
    pub async fn refresh_if_due(
        &self,
        providers: &[Arc<dyn DataProvider>],
        now_ms: i64,
    ) -> Vec<Exchange> {
        let mut updated = Vec::new();
        for exchange in Exchange::ALL {
            let last = self
                .store
                .setting(&refreshed_key(exchange))
                .ok()
                .flatten()
                .and_then(|v| v.parse::<i64>().ok());
            if last.is_some_and(|t| now_ms - t < REFRESH_AFTER_MS) {
                continue;
            }
            match self.refresh_exchange(exchange, providers, now_ms).await {
                Ok(n) => {
                    tracing::info!(
                        exchange = exchange.code(),
                        rows = n,
                        "instrument list refreshed"
                    );
                    updated.push(exchange);
                }
                Err(DataError::Unsupported) => {}
                Err(err) => {
                    tracing::warn!(exchange = exchange.code(), %err, "instrument refresh failed; keeping current list");
                }
            }
        }
        updated
    }

    async fn refresh_exchange(
        &self,
        exchange: Exchange,
        providers: &[Arc<dyn DataProvider>],
        now_ms: i64,
    ) -> Result<usize, DataError> {
        let mut last_err = DataError::Unsupported;
        for provider in providers {
            if !provider
                .capabilities()
                .contains(crate::Capabilities::INSTRUMENTS)
            {
                continue;
            }
            match provider.instruments(exchange).await {
                Ok(list) if list.len() >= self.min_rows => {
                    let rows: Vec<_> = list.iter().map(to_row).collect();
                    self.store
                        .replace_instruments(exchange.code(), &rows)
                        .map_err(store_err)?;
                    self.store
                        .set_setting(&refreshed_key(exchange), &now_ms.to_string())
                        .map_err(store_err)?;
                    self.rebuild()?;
                    return Ok(list.len());
                }
                Ok(list) => {
                    last_err = DataError::Parse(format!(
                        "instrument list too short ({} rows)",
                        list.len()
                    ));
                }
                Err(err) => last_err = err,
            }
        }
        Err(last_err)
    }

    fn rebuild(&self) -> Result<(), DataError> {
        let instruments = self
            .store
            .instruments()
            .map_err(store_err)?
            .into_iter()
            .filter_map(from_row)
            .collect();
        let tiers = match self.popularity.read() {
            Ok(guard) => Arc::clone(&guard),
            Err(poisoned) => Arc::clone(&poisoned.into_inner()),
        };
        let fresh = Arc::new(InstrumentIndex::new(instruments).with_popularity(&tiers));
        match self.index.write() {
            Ok(mut guard) => *guard = fresh,
            Err(poisoned) => *poisoned.into_inner() = fresh,
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use async_trait::async_trait;
    use nivesh_core::{FeedKind, ProviderId};

    use super::*;
    use crate::Capabilities;

    const FIXTURE: &str = include_str!("../tests/fixtures/nse/equity_list.csv");

    fn inst(sym: &str, name: &str) -> Instrument {
        Instrument {
            id: InstrumentId::new(Exchange::Nse, sym).unwrap(),
            name: name.into(),
            isin: None,
            series: Some("EQ".into()),
        }
    }

    fn syms(hits: &[Instrument]) -> Vec<&str> {
        hits.iter().map(|i| i.id.symbol()).collect()
    }

    fn sample() -> InstrumentIndex {
        InstrumentIndex::new(vec![
            inst("RELIANCE", "Reliance Industries Limited"),
            inst("RELINFRA", "Reliance Infrastructure Limited"),
            inst("RELIABLE", "Reliable Data Services Limited"),
            inst("RCOM", "Reliance Communications Limited"),
            inst("TCS", "Tata Consultancy Services Limited"),
            inst("TATAMOTORS", "Tata Motors Limited"),
            inst("TATASTEEL", "Tata Steel Limited"),
            inst("M&M", "Mahindra & Mahindra Limited"),
            inst("HDFCBANK", "HDFC Bank Limited"),
            inst("INFY", "Infosys Limited"),
            inst("ITC", "ITC Limited"),
            inst("IT", "Some IT Company"),
        ])
    }

    #[test]
    fn parses_the_real_nse_csv_format() {
        let list = parse_equity_csv(FIXTURE).unwrap();
        assert_eq!(list.len(), 9);
        let reliance = list.iter().find(|i| i.id.symbol() == "RELIANCE").unwrap();
        assert_eq!(reliance.name, "Reliance Industries Limited");
        assert_eq!(reliance.isin.as_deref(), Some("INE002A01018"));
        assert_eq!(reliance.series.as_deref(), Some("EQ"));
        assert!(list.iter().any(|i| i.id.symbol() == "M&M"));
        assert!(list.iter().any(|i| i.id.symbol() == "20MICRONS"));
    }

    #[test]
    fn csv_handles_quotes_blank_lines_and_bad_rows() {
        let text = "SYMBOL,NAME OF COMPANY,SERIES\nA,\"Acme, Inc.\",EQ\n\n,No Symbol,EQ\n!!,Bad Symbol,EQ\nB,\"Say \"\"hi\"\"\",BE\n";
        let list = parse_equity_csv(text).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "Acme, Inc.");
        assert_eq!(list[1].name, "Say \"hi\"");
        assert_eq!(list[1].series.as_deref(), Some("BE"));
        assert_eq!(list[0].isin, None);
    }

    #[test]
    fn csv_rejects_html_and_empty_input() {
        assert!(parse_equity_csv("").is_err());
        assert!(parse_equity_csv("<html><body>Access Denied</body></html>").is_err());
        assert!(parse_equity_csv("SYMBOL,NAME OF COMPANY\n").is_err());
    }

    #[test]
    fn the_bundled_seed_parses_and_has_the_blue_chips() {
        let seed = parse_equity_csv(SEED_NSE).unwrap();
        assert!(seed.len() > 2000, "{}", seed.len());
        for sym in ["RELIANCE", "TCS", "INFY", "HDFCBANK", "M&M", "BAJAJ-AUTO"] {
            assert!(seed.iter().any(|i| i.id.symbol() == sym), "{sym}");
        }
    }

    #[test]
    fn exact_symbol_beats_prefix_beats_name() {
        let idx = sample();
        assert_eq!(syms(&idx.search("tcs", 5))[0], "TCS");
        assert_eq!(
            syms(&idx.search("IT", 5))[..2],
            ["IT", "ITC"],
            "exact first, then prefix"
        );
        // "tata": nothing starts with it as a symbol prefix except TATA* ones;
        // those come before name-only matches such as TCS (Tata Consultancy).
        let tata = syms(&idx.search("tata", 5))
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(
            tata[..2],
            ["TATASTEEL", "TATAMOTORS"],
            "shorter symbol first: {tata:?}"
        );
        assert!(tata.contains(&"TCS".to_owned()));
    }

    fn tiers(pairs: &[(&str, u8)]) -> HashMap<String, u8> {
        pairs.iter().map(|(s, t)| ((*s).to_owned(), *t)).collect()
    }

    fn ranked() -> InstrumentIndex {
        sample().with_popularity(&tiers(&[
            ("RELIANCE", 0),
            ("TCS", 0),
            ("TATAMOTORS", 0),
            ("TATASTEEL", 1),
            ("INFY", 0),
            ("ITC", 0),
            ("RELINFRA", 2),
        ]))
    }

    #[test]
    fn without_popularity_ties_fall_back_to_symbol_order() {
        // RELIABLE and RELIANCE are both 8-letter prefixes: alphabetical.
        let hits = sample().search("relia", 5);
        assert_eq!(syms(&hits)[0], "RELIABLE");
        assert!(syms(&hits).contains(&"RELIANCE"));
    }

    #[test]
    fn relia_suggests_reliance_first_when_it_is_a_blue_chip() {
        let hits = ranked().search("relia", 5);
        assert_eq!(syms(&hits)[0], "RELIANCE", "{:?}", syms(&hits));
        // Symbol-prefix matches (RELIANCE, RELIABLE) come before name matches
        // (Reliance Infrastructure, Reliance Communications).
        assert_eq!(syms(&hits)[..2], ["RELIANCE", "RELIABLE"]);
        assert!(syms(&hits)[2..].contains(&"RELINFRA"));
        let hits = ranked().search("reliance", 5);
        assert_eq!(syms(&hits)[0], "RELIANCE", "exact symbol wins");
    }

    #[test]
    fn popularity_breaks_ties_but_never_beats_a_better_kind_of_match() {
        let idx = ranked();
        // Exact symbol beats a more popular prefix match.
        let hits = idx.search("it", 5);
        let it = syms(&hits);
        assert_eq!(it[..2], ["IT", "ITC"]);
        // A symbol prefix beats a popular name match: "tata" is a prefix of
        // two symbols; TCS (NIFTY 50) only matches by name, so it comes later.
        let hits = idx.search("tata", 5);
        let tata = syms(&hits);
        assert_eq!(
            tata[..2],
            ["TATAMOTORS", "TATASTEEL"],
            "NIFTY 50 before NEXT 50"
        );
        assert_eq!(tata[2], "TCS");
        // Among name matches the popular one is first.
        let hits = idx.search("reliance", 5);
        let reliance = syms(&hits);
        assert_eq!(reliance[0], "RELIANCE");
        assert_eq!(
            reliance[1], "RELINFRA",
            "ranked name match before the unranked RCOM"
        );
    }

    #[test]
    fn fuzzy_matches_prefer_tight_over_popular() {
        let idx = sample().with_popularity(&tiers(&[("TATASTEEL", 0)]));
        // "tmotors" only fits TATAMOTORS; popularity of others can't matter.
        assert_eq!(syms(&idx.search("tmotors", 5))[0], "TATAMOTORS");
    }

    #[test]
    fn bse_symbols_do_not_inherit_nse_popularity() {
        let bse_tcs = Instrument {
            id: InstrumentId::new(Exchange::Bse, "TCSX").unwrap(),
            name: "Other Co".into(),
            isin: None,
            series: None,
        };
        let idx = InstrumentIndex::new(vec![bse_tcs, inst("TCSL", "TCS Lookalike")])
            .with_popularity(&tiers(&[("TCSX", 0)]));
        assert_eq!(syms(&idx.search("tcs", 5)), ["TCSL", "TCSX"]);
    }

    #[test]
    fn name_words_match_in_any_order_and_prefix() {
        let idx = sample();
        assert_eq!(syms(&idx.search("tata mot", 3))[0], "TATAMOTORS");
        assert_eq!(
            syms(&idx.search("infrastructure reliance", 3))[0],
            "RELINFRA"
        );
        assert_eq!(syms(&idx.search("mahindra", 3))[0], "M&M");
    }

    #[test]
    fn symbols_with_punctuation_and_substrings() {
        let idx = sample();
        assert_eq!(syms(&idx.search("m&m", 3))[0], "M&M");
        assert!(
            syms(&idx.search("bank", 5)).contains(&"HDFCBANK"),
            "substring of symbol"
        );
    }

    #[test]
    fn fuzzy_matches_skipped_letters_but_only_for_three_plus_chars() {
        let idx = sample();
        // "tmotors" skips letters of TATAMOTORS' symbol.
        assert!(syms(&idx.search("tmotors", 5)).contains(&"TATAMOTORS"));
        // Two letters are prefix-only, never fuzzy.
        assert!(idx.search("hk", 5).is_empty());
        assert!(syms(&idx.search("hfcb", 5)).contains(&"HDFCBANK"));
    }

    #[test]
    fn empty_and_unmatched_queries_return_nothing() {
        let idx = sample();
        assert!(idx.search("", 5).is_empty());
        assert!(idx.search("   ", 5).is_empty());
        assert!(idx.search("zzzzzz", 5).is_empty());
        assert!(idx.search("tcs", 0).is_empty());
    }

    #[test]
    fn search_is_case_insensitive_and_respects_the_limit() {
        let idx = sample();
        assert_eq!(idx.search("RELIANCE", 5), idx.search("reliance", 5));
        assert_eq!(idx.search("rel", 2).len(), 2);
    }

    #[test]
    fn search_over_the_full_seed_is_far_inside_the_budget() {
        let mut all = parse_equity_csv(SEED_NSE).unwrap();
        all.extend(parse_list_csv(SEED_BSE, Exchange::Bse).unwrap());
        let tiers = ConstituentSet::bundled().unwrap().popularity();
        let idx = InstrumentIndex::new(all).with_popularity(&tiers);
        let started = Instant::now();
        let mut hits = Vec::new();
        for q in ["relia", "tata mot", "hdfc", "x", "zzzzz", "bajaj"] {
            hits = idx.search(q, 10);
            assert!(
                started.elapsed().as_millis() < 500,
                "debug builds are slow, but not that slow"
            );
        }
        let per_query = started.elapsed() / 6;
        // The acceptance budget is 50 ms, in debug builds.
        assert!(per_query.as_millis() < 50, "{per_query:?} per query");
        assert!(!hits.is_empty());
        let relia = idx.search("relia", 10);
        assert!(syms(&relia).contains(&"RELIANCE"), "{:?}", syms(&relia));
        assert_eq!(syms(&idx.search("reliance", 1)), ["RELIANCE"]);
    }

    /// The full NSE list with the bundled NIFTY 500 tiers: the blue chip a
    /// trader means is first for the usual partial typings.
    fn real_ranked() -> InstrumentIndex {
        let tiers = ConstituentSet::bundled().unwrap().popularity();
        InstrumentIndex::new(parse_equity_csv(SEED_NSE).unwrap()).with_popularity(&tiers)
    }

    #[test]
    fn on_the_real_list_relia_ranks_reliance_first() {
        let idx = real_ranked();
        for q in ["relia", "RELIA", "reli", "reliance", "Reliance Ind"] {
            let hits = idx.search(q, 8);
            assert_eq!(syms(&hits)[0], "RELIANCE", "{q}: {:?}", syms(&hits));
        }
    }

    #[test]
    fn on_the_real_list_common_partials_find_the_blue_chip() {
        let idx = real_ranked();
        for (q, want) in [
            ("hdfcb", "HDFCBANK"),
            ("sbi", "SBIN"),
            ("infy", "INFY"),
            ("icici", "ICICIBANK"),
            ("tcs", "TCS"),
            ("bajaj-a", "BAJAJ-AUTO"),
            ("maruti", "MARUTI"),
        ] {
            let hits = idx.search(q, 8);
            assert_eq!(syms(&hits)[0], want, "{q}: {:?}", syms(&hits));
        }
    }

    #[test]
    fn master_ranks_with_the_bundled_tiers_and_can_be_re_ranked() {
        let master = InstrumentMaster::open(Arc::new(Store::open_in_memory().unwrap())).unwrap();
        assert_eq!(syms(&master.search("relia", 3))[0], "RELIANCE");
        // Demote everything: back to plain symbol order, RELIABLE first.
        master.set_popularity(HashMap::new()).unwrap();
        assert_eq!(syms(&master.search("relia", 3))[0], "RELIABLE");
        master.set_popularity(tiers(&[("RELIANCE", 0)])).unwrap();
        assert_eq!(syms(&master.search("relia", 3))[0], "RELIANCE");
    }

    // ---- master ----

    struct ListProvider(Result<Vec<Instrument>, DataError>);

    #[async_trait]
    impl DataProvider for ListProvider {
        fn id(&self) -> ProviderId {
            ProviderId::Nse
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities::INSTRUMENTS
        }
        fn feed_kind(&self) -> FeedKind {
            FeedKind::Delayed
        }
        async fn instruments(&self, exchange: Exchange) -> Result<Vec<Instrument>, DataError> {
            if exchange == Exchange::Nse {
                self.0.clone()
            } else {
                Err(DataError::Unsupported)
            }
        }
    }

    fn provider(list: Result<Vec<Instrument>, DataError>) -> Vec<Arc<dyn DataProvider>> {
        vec![Arc::new(ListProvider(list))]
    }

    #[test]
    fn first_run_seeds_nse_and_is_searchable_offline() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let master = InstrumentMaster::open(Arc::clone(&store)).unwrap();
        assert!(master.len() > 2000);
        let hits = master.search("relia", 5);
        assert!(syms(&hits).contains(&"RELIANCE"));
        // The seed was written to the store, so the second start reads it back.
        assert!(store.instruments().unwrap().len() > 2000);
        let again = InstrumentMaster::open(store).unwrap();
        assert_eq!(again.len(), master.len());
    }

    #[tokio::test]
    async fn a_due_refresh_replaces_the_list_and_stamps_the_time() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let master = InstrumentMaster::open(Arc::clone(&store))
            .unwrap()
            .with_min_rows(2);
        let fresh = vec![
            inst("NEWCO", "New Company Limited"),
            inst("OLDCO", "Old Company Limited"),
        ];
        let updated = master.refresh_if_due(&provider(Ok(fresh)), 1_000).await;
        assert_eq!(updated, [Exchange::Nse]);
        // NSE now has two rows; the seeded BSE list is untouched.
        assert_eq!(
            master
                .get(&InstrumentId::parse("NEWCO").unwrap())
                .unwrap()
                .name,
            "New Company Limited"
        );
        assert!(
            master
                .get(&InstrumentId::parse("RELIANCE").unwrap())
                .is_none()
        );
        assert!(
            master
                .get(&InstrumentId::parse("BSE:ABB").unwrap())
                .is_some()
        );
        assert_eq!(syms(&master.search("newco", 5))[0], "NEWCO");
        assert_eq!(
            store
                .setting("instruments.refreshed_at.NSE")
                .unwrap()
                .as_deref(),
            Some("1000")
        );
    }

    #[tokio::test]
    async fn refresh_is_skipped_within_a_day_and_runs_again_after() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let master = InstrumentMaster::open(store).unwrap().with_min_rows(1);
        let one = || provider(Ok(vec![inst("ONE", "One Limited")]));
        assert_eq!(master.refresh_if_due(&one(), 0).await.len(), 1);
        assert!(
            master
                .refresh_if_due(&one(), REFRESH_AFTER_MS - 1)
                .await
                .is_empty()
        );
        assert_eq!(
            master.refresh_if_due(&one(), REFRESH_AFTER_MS).await.len(),
            1
        );
    }

    #[tokio::test]
    async fn a_failed_or_implausible_download_keeps_the_current_list() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let master = InstrumentMaster::open(store).unwrap();
        let before = master.len();
        assert!(
            master
                .refresh_if_due(&provider(Err(DataError::Blocked)), 5)
                .await
                .is_empty()
        );
        assert_eq!(master.len(), before);
        // Three rows is below the plausibility floor: not a real list.
        let tiny = vec![inst("A", "A Ltd"), inst("B", "B Ltd"), inst("C", "C Ltd")];
        assert!(
            master
                .refresh_if_due(&provider(Ok(tiny)), 5)
                .await
                .is_empty()
        );
        assert_eq!(master.len(), before);
        // And the failed attempt didn't stamp a refresh time: it is retried.
        assert!(
            master
                .store
                .setting("instruments.refreshed_at.NSE")
                .unwrap()
                .is_none()
        );
    }

    // ---- BSE ----

    fn bse(sym: &str, name: &str, isin: Option<&str>) -> Instrument {
        Instrument {
            id: InstrumentId::new(Exchange::Bse, sym).unwrap(),
            name: name.into(),
            isin: isin.map(str::to_owned),
            series: Some("A".into()),
        }
    }

    fn nse_isin(sym: &str, name: &str, isin: &str) -> Instrument {
        Instrument {
            isin: Some(isin.into()),
            ..inst(sym, name)
        }
    }

    fn mixed() -> InstrumentIndex {
        InstrumentIndex::new(vec![
            nse_isin("RELIANCE", "Reliance Industries Limited", "INE002A01018"),
            bse("RELIANCE", "RELIANCE INDUSTRIES LTD.", Some("INE002A01018")),
            nse_isin("TCS", "Tata Consultancy Services Limited", "INE467B01029"),
            bse(
                "532540",
                "TATA CONSULTANCY SERVICES LTD.",
                Some("INE467B01029"),
            ),
            bse("SAMOR", "Samor Reality Limited", Some("INE0GD201022")),
            bse("NOISIN", "No Isin Limited", None),
            // Same ticker as an NSE stock, but a different company.
            nse_isin(
                "KALYANI",
                "Kalyani Investment Company Limited",
                "INE029L01018",
            ),
            bse("KALYANI", "Kalyani Cast-Tech Limited", Some("INE0KCT01010")),
        ])
    }

    fn tags(hits: &[Instrument]) -> Vec<String> {
        hits.iter().map(|i| i.id.to_string()).collect()
    }

    #[test]
    fn bse_only_stocks_are_found_without_a_prefix() {
        let hits = mixed().search("samor", 5);
        assert_eq!(tags(&hits), ["BSE:SAMOR"]);
        assert_eq!(
            tags(&mixed().search("noisin", 5)),
            ["BSE:NOISIN"],
            "no ISIN: can't be matched, stays"
        );
        assert_eq!(
            tags(&mixed().search("reality", 5)),
            ["BSE:SAMOR"],
            "by name"
        );
    }

    #[test]
    fn dual_listed_stocks_show_once_as_the_nse_listing() {
        let idx = mixed();
        assert_eq!(tags(&idx.search("reliance", 5)), ["NSE:RELIANCE"]);
        assert_eq!(tags(&idx.search("tata consultancy", 5)), ["NSE:TCS"]);
        // The hidden BSE listing is still there for lookups by id.
        let bse_rel = InstrumentId::new(Exchange::Bse, "RELIANCE").unwrap();
        assert!(idx.get(&bse_rel).is_some());
        assert_eq!(idx.len(), 8);
    }

    #[test]
    fn same_ticker_different_company_is_not_deduplicated() {
        let hits = mixed().search("kalyani", 5);
        assert_eq!(tags(&hits), ["NSE:KALYANI", "BSE:KALYANI"], "NSE first");
    }

    #[test]
    fn the_bundled_bse_seed_parses_and_has_bse_only_names() {
        let seed = parse_list_csv(SEED_BSE, Exchange::Bse).unwrap();
        assert!(seed.len() > 3000, "{}", seed.len());
        assert!(seed.iter().all(|i| i.id.exchange() == Exchange::Bse));
        assert!(
            seed.iter()
                .all(|i| i.isin.as_deref().is_some_and(|s| s.starts_with("INE")))
        );
        for sym in ["RELIANCE", "TCS", "ABB", "SAMOR"] {
            assert!(seed.iter().any(|i| i.id.symbol() == sym), "{sym}");
        }
    }

    #[test]
    fn with_both_seeds_bse_only_stocks_search_and_dual_listings_collapse() {
        let master = InstrumentMaster::open(Arc::new(Store::open_in_memory().unwrap())).unwrap();
        // A BSE-only company is found by ticker and by name, no "BSE:" needed.
        let samor = master.search("samor", 5);
        assert_eq!(tags(&samor)[0], "BSE:SAMOR", "{:?}", tags(&samor));
        assert_eq!(tags(&master.search("samor reality", 3))[0], "BSE:SAMOR");
        // A dual-listed blue chip appears once, as NSE.
        let rel = master.search("reliance", 10);
        assert_eq!(tags(&rel)[0], "NSE:RELIANCE");
        assert!(
            !tags(&rel).contains(&"BSE:RELIANCE".to_owned()),
            "{:?}",
            tags(&rel)
        );
        // "relia" still puts the blue chip first with BSE rows in the index.
        assert_eq!(tags(&master.search("relia", 5))[0], "NSE:RELIANCE");
        // Nothing appears twice for the same ISIN.
        for q in ["tata", "bank", "hdfc", "ad"] {
            let hits = master.search(q, 40);
            let mut isins: Vec<_> = hits.iter().filter_map(|i| i.isin.clone()).collect();
            let n = isins.len();
            isins.sort();
            isins.dedup();
            assert_eq!(
                isins.len(),
                n,
                "{q}: duplicate company in {:?}",
                tags(&hits)
            );
        }
    }

    #[test]
    fn an_install_that_only_has_nse_gets_the_bse_seed_on_next_start() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let nse_only: Vec<_> = parse_equity_csv(SEED_NSE)
            .unwrap()
            .iter()
            .map(to_row)
            .collect();
        store.replace_instruments("NSE", &nse_only).unwrap();
        let master = InstrumentMaster::open(Arc::clone(&store)).unwrap();
        assert_eq!(tags(&master.search("samor", 3))[0], "BSE:SAMOR");
        assert!(
            store
                .instruments()
                .unwrap()
                .iter()
                .any(|r| r.exchange == "BSE")
        );
    }

    struct BseProvider(Result<Vec<Instrument>, DataError>);

    #[async_trait]
    impl DataProvider for BseProvider {
        fn id(&self) -> ProviderId {
            ProviderId::Bse
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities::INSTRUMENTS
        }
        fn feed_kind(&self) -> FeedKind {
            FeedKind::Delayed
        }
        async fn instruments(&self, exchange: Exchange) -> Result<Vec<Instrument>, DataError> {
            if exchange == Exchange::Bse {
                self.0.clone()
            } else {
                Err(DataError::Unsupported)
            }
        }
    }

    #[tokio::test]
    async fn a_bse_refresh_replaces_only_the_bse_list_and_a_block_keeps_the_old_one() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let master = InstrumentMaster::open(Arc::clone(&store))
            .unwrap()
            .with_min_rows(1);
        let blocked: Vec<Arc<dyn DataProvider>> =
            vec![Arc::new(BseProvider(Err(DataError::Blocked)))];
        assert!(master.refresh_if_due(&blocked, 10).await.is_empty());
        assert!(
            master
                .get(&InstrumentId::parse("BSE:SAMOR").unwrap())
                .is_some()
        );

        let fresh: Vec<Arc<dyn DataProvider>> = vec![Arc::new(BseProvider(Ok(vec![bse(
            "NEWBSE",
            "New Bse Limited",
            Some("INE0NEW01011"),
        )])))];
        assert_eq!(master.refresh_if_due(&fresh, 10).await, [Exchange::Bse]);
        assert!(
            master
                .get(&InstrumentId::parse("BSE:SAMOR").unwrap())
                .is_none()
        );
        assert_eq!(tags(&master.search("newbse", 3))[0], "BSE:NEWBSE");
        assert!(
            master
                .get(&InstrumentId::parse("RELIANCE").unwrap())
                .is_some(),
            "NSE untouched"
        );
        assert!(
            store
                .setting("instruments.refreshed_at.BSE")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn lookup_by_id() {
        let master = InstrumentMaster::open(Arc::new(Store::open_in_memory().unwrap())).unwrap();
        let id = InstrumentId::parse("RELIANCE").unwrap();
        assert_eq!(master.get(&id).unwrap().name, "Reliance Industries Limited");
        assert!(
            master
                .get(&InstrumentId::parse("NOPE123").unwrap())
                .is_none()
        );
    }
}
