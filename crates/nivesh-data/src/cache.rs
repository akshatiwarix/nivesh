//! Two-tier caching with stale-while-revalidate.
//!
//! Tier 1 is an in-memory `moka` map; tier 2 is the persistent store (SQLite).
//! Each entry has a *fresh* window (served as is) followed by a *revalidate*
//! window (served immediately while a refresh runs in the background). Past
//! that, the next read refreshes in the foreground, and if every provider
//! fails the old value is returned flagged **STALE**, from memory or, after a
//! restart, from the store. So offline starts show the last known data rather
//! than nothing, and never as if it were current.

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, TimeZone as _, Utc};
use futures::FutureExt as _;
use futures::future::BoxFuture;
use moka::sync::Cache;
use nivesh_core::{CandleSeries, IndexId, IndexQuote, IndexSnapshot, InstrumentId, Quote, Sourced};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::clock::Clock;
use crate::{DataError, HistoryRequest, MarketData, Movers, MoversRequest};

/// How long an entry is fresh, and how long after that it may still be served
/// while it is refreshed in the background.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ttl {
    pub fresh: Duration,
    pub revalidate: Duration,
}

impl Ttl {
    pub const fn new(fresh: Duration, revalidate: Duration) -> Self {
        Self { fresh, revalidate }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CachePolicy {
    pub quote: Ttl,
    pub index: Ttl,
    pub all_indices: Ttl,
    pub intraday_history: Ttl,
    /// Daily history is fresh until the next session close; this is how long
    /// it may then be served while revalidating.
    pub daily_history_revalidate: Duration,
    /// Quotes change on every poll; persisting each one would just churn the
    /// disk, so a given key is written at most this often.
    pub quote_persist_every: Duration,
}

impl Default for CachePolicy {
    fn default() -> Self {
        let s = Duration::from_secs;
        Self {
            quote: Ttl::new(s(10), s(30)),
            index: Ttl::new(s(15), s(45)),
            all_indices: Ttl::new(s(15), s(45)),
            intraday_history: Ttl::new(s(60), s(300)),
            daily_history_revalidate: s(3600),
            quote_persist_every: s(30),
        }
    }
}

/// The persistent tier. `nivesh_store::Store` implements it; tests use a map.
pub trait PersistentCache: Send + Sync + 'static {
    /// Returns the payload and when it was fetched (UTC epoch ms).
    fn get(&self, key: &str) -> Option<(String, i64)>;
    fn put(&self, key: &str, kind: &str, payload: &str, fetched_at_ms: i64);
}

impl PersistentCache for nivesh_store::Store {
    fn get(&self, key: &str) -> Option<(String, i64)> {
        match self.cache_get(key) {
            Ok(row) => row.map(|r| (r.payload, r.fetched_at_ms)),
            Err(err) => {
                tracing::warn!(%err, "data cache read failed");
                None
            }
        }
    }

    fn put(&self, key: &str, kind: &str, payload: &str, fetched_at_ms: i64) {
        if let Err(err) = self.cache_put(key, kind, payload, fetched_at_ms) {
            tracing::warn!(%err, "data cache write failed");
        }
    }
}

#[derive(Clone)]
pub struct CacheConfig {
    pub policy: CachePolicy,
    pub persistent: Option<Arc<dyn PersistentCache>>,
    pub max_entries: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            policy: CachePolicy::default(),
            persistent: None,
            max_entries: 5000,
        }
    }
}

/// Marks a value as served from stale cache.
pub trait MarkStale {
    fn mark_stale(&mut self);
}

impl<T> MarkStale for Sourced<T> {
    fn mark_stale(&mut self) {
        self.provenance.stale = true;
    }
}

impl<T> MarkStale for Vec<Sourced<T>> {
    fn mark_stale(&mut self) {
        self.iter_mut().for_each(MarkStale::mark_stale);
    }
}

struct Entry {
    value: Arc<dyn Any + Send + Sync>,
    stored: Duration,
    ttl: Ttl,
}

enum Lookup<V> {
    Fresh(V),
    /// Past its fresh window but still servable while revalidating.
    Soft(V),
    Expired,
    Miss,
}

pub(crate) struct CacheLayer {
    mem: Cache<String, Arc<Entry>>,
    persistent: Option<Arc<dyn PersistentCache>>,
    clock: Arc<dyn Clock>,
    pub(crate) policy: CachePolicy,
    persisted_at: Mutex<HashMap<String, Duration>>,
    refreshing: Mutex<HashSet<String>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl CacheLayer {
    pub(crate) fn new(config: CacheConfig, clock: Arc<dyn Clock>) -> Self {
        Self {
            mem: Cache::new(config.max_entries),
            persistent: config.persistent,
            clock,
            policy: config.policy,
            persisted_at: Mutex::new(HashMap::new()),
            refreshing: Mutex::new(HashSet::new()),
        }
    }

    fn peek<V: Clone + 'static>(&self, key: &str) -> Lookup<V> {
        let Some(entry) = self.mem.get(key) else {
            return Lookup::Miss;
        };
        let Some(value) = entry.value.downcast_ref::<V>() else {
            return Lookup::Miss;
        };
        let age = self.clock.now().saturating_sub(entry.stored);
        if age <= entry.ttl.fresh {
            Lookup::Fresh(value.clone())
        } else if age <= entry.ttl.fresh + entry.ttl.revalidate {
            Lookup::Soft(value.clone())
        } else {
            Lookup::Expired
        }
    }

    fn put<V>(&self, key: &str, kind: &str, ttl: Ttl, value: &V, persist_every: Duration)
    where
        V: Clone + Serialize + Send + Sync + 'static,
    {
        let now = self.clock.now();
        self.mem.insert(
            key.to_owned(),
            Arc::new(Entry {
                value: Arc::new(value.clone()),
                stored: now,
                ttl,
            }),
        );
        let Some(store) = &self.persistent else {
            return;
        };
        {
            let mut persisted = lock(&self.persisted_at);
            let due = persisted
                .get(key)
                .is_none_or(|last| now.saturating_sub(*last) >= persist_every);
            if !due {
                return;
            }
            persisted.insert(key.to_owned(), now);
        }
        match serde_json::to_string(value) {
            Ok(json) => store.put(key, kind, &json, Utc::now().timestamp_millis()),
            Err(err) => tracing::warn!(%err, key, "cannot serialise cache entry"),
        }
    }

    /// The last known value of any age, from memory or the store, flagged
    /// stale. Used only when fetching failed.
    fn stale<V>(&self, key: &str) -> Option<V>
    where
        V: Clone + DeserializeOwned + MarkStale + 'static,
    {
        let mut value = match self.mem.get(key) {
            Some(entry) => entry.value.downcast_ref::<V>()?.clone(),
            None => {
                let (payload, _) = self.persistent.as_ref()?.get(key)?;
                serde_json::from_str::<V>(&payload).ok()?
            }
        };
        value.mark_stale();
        Some(value)
    }

    /// Claims the right to refresh `key` in the background. False if someone
    /// else already is.
    fn begin_refresh(&self, key: &str) -> bool {
        lock(&self.refreshing).insert(key.to_owned())
    }

    fn end_refresh(&self, key: &str) {
        lock(&self.refreshing).remove(key);
    }
}

type Fetch<V> = Arc<dyn Fn() -> BoxFuture<'static, Result<V, DataError>> + Send + Sync>;

/// Serves one keyed value with the fresh / revalidate / refresh / stale logic.
async fn get_or_fetch<V>(
    layer: &Arc<CacheLayer>,
    key: String,
    kind: &'static str,
    ttl: Ttl,
    fetch: Fetch<V>,
) -> Result<V, DataError>
where
    V: Clone + Serialize + DeserializeOwned + MarkStale + Send + Sync + 'static,
{
    match layer.peek::<V>(&key) {
        Lookup::Fresh(v) => return Ok(v),
        Lookup::Soft(v) => {
            let (layer, key, fetch) = (Arc::clone(layer), key.clone(), Arc::clone(&fetch));
            if layer.begin_refresh(&key) {
                tokio::spawn(async move {
                    if let Ok(fresh) = fetch().await {
                        layer.put(&key, kind, ttl, &fresh, Duration::ZERO);
                    }
                    layer.end_refresh(&key);
                });
            }
            return Ok(v);
        }
        Lookup::Expired | Lookup::Miss => {}
    }
    match fetch().await {
        Ok(v) => {
            layer.put(&key, kind, ttl, &v, Duration::ZERO);
            Ok(v)
        }
        Err(err) => layer.stale::<V>(&key).ok_or(err),
    }
}

fn quote_key(id: &InstrumentId) -> String {
    format!("q:{id}")
}

pub(crate) async fn quotes(
    md: &MarketData,
    layer: &Arc<CacheLayer>,
    ids: &[InstrumentId],
) -> Result<Vec<Sourced<Quote>>, DataError> {
    let ttl = layer.policy.quote;
    let persist_every = layer.policy.quote_persist_every;
    let mut wanted: Vec<InstrumentId> = Vec::with_capacity(ids.len());
    for id in ids {
        if !wanted.contains(id) {
            wanted.push(id.clone());
        }
    }
    if wanted.is_empty() {
        return Ok(Vec::new());
    }

    let mut found: HashMap<InstrumentId, Sourced<Quote>> = HashMap::new();
    let (mut need, mut soft) = (Vec::new(), Vec::new());
    for id in &wanted {
        match layer.peek::<Sourced<Quote>>(&quote_key(id)) {
            Lookup::Fresh(q) => {
                found.insert(id.clone(), q);
            }
            Lookup::Soft(q) => {
                found.insert(id.clone(), q);
                soft.push(id.clone());
            }
            Lookup::Expired | Lookup::Miss => need.push(id.clone()),
        }
    }

    if !soft.is_empty() {
        let refresh_key = format!(
            "batch:{}",
            soft.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",")
        );
        if layer.begin_refresh(&refresh_key) {
            let (md, layer) = (md.clone(), Arc::clone(layer));
            tokio::spawn(async move {
                if let Ok(fresh) = md.fetch_quotes(&soft).await {
                    for q in &fresh {
                        layer.put(&quote_key(&q.data.id), "quote", ttl, q, persist_every);
                    }
                }
                layer.end_refresh(&refresh_key);
            });
        }
    }

    if !need.is_empty() {
        match md.fetch_quotes(&need).await {
            Ok(fresh) => {
                for q in fresh {
                    layer.put(&quote_key(&q.data.id), "quote", ttl, &q, persist_every);
                    found.insert(q.data.id.clone(), q);
                }
            }
            Err(err) => {
                for id in &need {
                    if let Some(q) = layer.stale::<Sourced<Quote>>(&quote_key(id)) {
                        found.insert(id.clone(), q);
                    }
                }
                if found.is_empty() {
                    return Err(err);
                }
            }
        }
    }

    let out: Vec<_> = wanted.iter().filter_map(|id| found.remove(id)).collect();
    if out.is_empty() {
        Err(DataError::NotFound)
    } else {
        Ok(out)
    }
}

pub(crate) async fn index_snapshot(
    md: &MarketData,
    layer: &Arc<CacheLayer>,
    index: IndexId,
) -> Result<Sourced<IndexSnapshot>, DataError> {
    let md = md.clone();
    let fetch: Fetch<Sourced<IndexSnapshot>> = Arc::new(move || {
        let md = md.clone();
        async move { md.fetch_index_snapshot(index).await }.boxed()
    });
    get_or_fetch(
        layer,
        format!("i:{}", index.code()),
        "index",
        layer.policy.index,
        fetch,
    )
    .await
}

pub(crate) async fn all_indices(
    md: &MarketData,
    layer: &Arc<CacheLayer>,
) -> Result<Vec<Sourced<IndexQuote>>, DataError> {
    let md = md.clone();
    let fetch: Fetch<Vec<Sourced<IndexQuote>>> = Arc::new(move || {
        let md = md.clone();
        async move { md.fetch_all_indices().await }.boxed()
    });
    get_or_fetch(
        layer,
        "ai".to_owned(),
        "index",
        layer.policy.all_indices,
        fetch,
    )
    .await
}

pub(crate) async fn movers(
    md: &MarketData,
    layer: &Arc<CacheLayer>,
    req: &MoversRequest,
) -> Result<Sourced<Movers>, DataError> {
    let (md, request) = (md.clone(), *req);
    let fetch: Fetch<Sourced<Movers>> = Arc::new(move || {
        let md = md.clone();
        async move { md.fetch_movers(&request).await }.boxed()
    });
    get_or_fetch(
        layer,
        format!("m:{}:{}", req.index.code(), req.limit),
        "movers",
        layer.policy.index,
        fetch,
    )
    .await
}

/// NSE closes at 15:30 IST = 10:00 UTC.
const CLOSE_UTC_HOUR: u32 = 10;

/// How long daily candles stay fresh: until the next session close (with a
/// floor so a request just after close isn't refetched every call).
pub fn daily_fresh_for(now: DateTime<Utc>) -> Duration {
    let today_close = Utc
        .with_ymd_and_hms(
            chrono::Datelike::year(&now),
            chrono::Datelike::month(&now),
            chrono::Datelike::day(&now),
            CLOSE_UTC_HOUR,
            0,
            0,
        )
        .single()
        .unwrap_or(now);
    let next = if today_close > now {
        today_close
    } else {
        today_close + ChronoDuration::days(1)
    };
    (next - now)
        .to_std()
        .unwrap_or_default()
        .max(Duration::from_secs(60))
}

pub(crate) async fn history(
    md: &MarketData,
    layer: &Arc<CacheLayer>,
    req: &HistoryRequest,
) -> Result<Sourced<CandleSeries>, DataError> {
    let ttl = if req.interval.is_intraday() {
        layer.policy.intraday_history
    } else {
        Ttl::new(
            daily_fresh_for(Utc::now()),
            layer.policy.daily_history_revalidate,
        )
    };
    let key = format!(
        "h:{}:{}:{}",
        req.id,
        req.interval,
        serde_json::to_string(&req.range).unwrap_or_default()
    );
    let (md, request) = (md.clone(), req.clone());
    let fetch: Fetch<Sourced<CandleSeries>> = Arc::new(move || {
        let (md, request) = (md.clone(), request.clone());
        async move { md.fetch_history(&request).await }.boxed()
    });
    get_or_fetch(layer, key, "history", ttl, fetch).await
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use nivesh_core::{Candle, FeedKind, Freshness, Interval, Provenance, ProviderId};

    use super::*;
    use crate::clock::ManualClock;
    use crate::mock::MockProvider;
    use crate::{Capabilities, HistoryRange};

    #[derive(Default)]
    struct MemPersist(Mutex<HashMap<String, (String, i64)>>);

    impl PersistentCache for MemPersist {
        fn get(&self, key: &str) -> Option<(String, i64)> {
            lock(&self.0).get(key).cloned()
        }
        fn put(&self, key: &str, _kind: &str, payload: &str, at: i64) {
            lock(&self.0).insert(key.to_owned(), (payload.to_owned(), at));
        }
    }

    fn id(s: &str) -> InstrumentId {
        InstrumentId::parse(s).unwrap()
    }

    fn quote(sym: &str, ltp: f64) -> Sourced<Quote> {
        let now = Utc::now();
        Sourced::new(
            Quote {
                id: id(sym),
                name: None,
                ltp,
                prev_close: 100.0,
                open: None,
                high: None,
                low: None,
                volume: None,
                week52_high: None,
                week52_low: None,
                upper_band: None,
                lower_band: None,
            },
            Provenance::new(ProviderId::Nse, FeedKind::Delayed, now, now),
        )
    }

    struct Rig {
        md: MarketData,
        p: Arc<MockProvider>,
        clock: Arc<ManualClock>,
        persist: Arc<MemPersist>,
    }

    fn rig() -> Rig {
        let p = Arc::new(MockProvider::new(
            ProviderId::Nse,
            Capabilities::QUOTE | Capabilities::INDEX | Capabilities::HISTORY,
        ));
        let clock = Arc::new(ManualClock::new());
        let persist = Arc::new(MemPersist::default());
        let md = MarketData::builder()
            .provider(p.clone())
            .clock(clock.clone())
            .cache(CacheConfig {
                persistent: Some(persist.clone()),
                ..CacheConfig::default()
            })
            .build();
        Rig {
            md,
            p,
            clock,
            persist,
        }
    }

    const S: fn(u64) -> Duration = Duration::from_secs;

    #[tokio::test]
    async fn fresh_entries_are_served_without_a_provider_call() {
        let r = rig();
        r.p.repeat_quotes(Ok(vec![quote("TCS", 10.0)]));
        r.md.quote(&id("TCS")).await.unwrap();
        r.clock.advance(S(5));
        let q = r.md.quote(&id("TCS")).await.unwrap();
        assert_eq!(q.data.ltp, 10.0);
        assert_eq!(r.p.quote_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn expired_entries_are_served_at_once_and_refreshed_in_the_background() {
        let r = rig();
        r.p.repeat_quotes(Ok(vec![quote("TCS", 10.0)]));
        r.md.quote(&id("TCS")).await.unwrap();
        r.p.repeat_quotes(Ok(vec![quote("TCS", 11.0)]));
        r.clock.advance(S(20)); // past fresh (10 s), within revalidate (30 s)
        let q = r.md.quote(&id("TCS")).await.unwrap();
        assert_eq!(q.data.ltp, 10.0, "old value returned immediately");
        assert_eq!(
            q.provenance.freshness(),
            Freshness::Delayed,
            "not stale: it is being refreshed"
        );
        // The background refresh lands shortly after.
        for _ in 0..50 {
            if r.p.quote_calls.load(Ordering::SeqCst) >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(r.md.quote(&id("TCS")).await.unwrap().data.ltp, 11.0);
        assert_eq!(r.p.quote_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn past_the_revalidate_window_it_refreshes_in_the_foreground() {
        let r = rig();
        r.p.repeat_quotes(Ok(vec![quote("TCS", 10.0)]));
        r.md.quote(&id("TCS")).await.unwrap();
        r.p.repeat_quotes(Ok(vec![quote("TCS", 12.0)]));
        r.clock.advance(S(120));
        assert_eq!(r.md.quote(&id("TCS")).await.unwrap().data.ltp, 12.0);
    }

    #[tokio::test]
    async fn offline_returns_the_cached_value_flagged_stale() {
        let r = rig();
        r.p.repeat_quotes(Ok(vec![quote("TCS", 10.0)]));
        r.md.quote(&id("TCS")).await.unwrap();
        r.p.repeat_quotes(Err(DataError::Upstream("offline".into())));
        r.clock.advance(S(600));
        let q = r.md.quote(&id("TCS")).await.unwrap();
        assert_eq!(q.data.ltp, 10.0);
        assert_eq!(q.provenance.freshness(), Freshness::Stale);
        assert!(q.provenance.stale);
    }

    #[tokio::test]
    async fn an_offline_cold_start_reads_the_persistent_store_as_stale() {
        let r = rig();
        r.p.repeat_quotes(Ok(vec![quote("TCS", 10.0)]));
        r.md.quote(&id("TCS")).await.unwrap();
        assert!(r.persist.get("q:NSE:TCS").is_some(), "persisted");

        // "Restart": a new facade with empty memory, same store, no network.
        let p2 = Arc::new(MockProvider::new(ProviderId::Nse, Capabilities::QUOTE));
        p2.repeat_quotes(Err(DataError::Timeout));
        let md2 = MarketData::builder()
            .provider(p2)
            .cache(CacheConfig {
                persistent: Some(r.persist.clone()),
                ..CacheConfig::default()
            })
            .build();
        let q = md2.quote(&id("TCS")).await.unwrap();
        assert_eq!(q.data.ltp, 10.0);
        assert_eq!(q.provenance.freshness(), Freshness::Stale);
    }

    #[tokio::test]
    async fn nothing_cached_and_offline_is_an_error_not_a_crash() {
        let r = rig();
        r.p.repeat_quotes(Err(DataError::Timeout));
        assert_eq!(
            r.md.quote(&id("TCS")).await.unwrap_err(),
            DataError::Timeout
        );
    }

    #[tokio::test]
    async fn a_batch_only_fetches_what_is_missing() {
        let r = rig();
        r.p.repeat_quotes(Ok(vec![quote("TCS", 10.0)]));
        r.md.quote(&id("TCS")).await.unwrap();
        r.p.repeat_quotes(Ok(vec![quote("INFY", 20.0)]));
        let both = r.md.quotes(&[id("INFY"), id("TCS")]).await.unwrap();
        let syms: Vec<_> = both.iter().map(|q| q.data.id.symbol().to_owned()).collect();
        assert_eq!(syms, ["INFY", "TCS"], "request order is kept");
        assert_eq!(r.p.quote_requests().last().unwrap(), &vec![id("INFY")]);
    }

    /// Waits until the provider has been called `n` times and the refresh
    /// that made the call has had a moment to store its result.
    async fn wait_for_calls(p: &MockProvider, n: usize) {
        for _ in 0..100 {
            if p.quote_calls.load(Ordering::SeqCst) >= n {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    #[tokio::test]
    async fn persisting_quotes_is_throttled_per_key() {
        let r = rig();
        r.p.repeat_quotes(Ok(vec![quote("TCS", 10.0)]));
        r.md.quote(&id("TCS")).await.unwrap(); // stored and persisted at t=0

        // t=15 s: served from cache, refreshed in the background. The refresh
        // updates memory but is not persisted again within 30 s.
        r.clock.advance(S(15));
        r.p.repeat_quotes(Ok(vec![quote("TCS", 11.0)]));
        assert_eq!(r.md.quote(&id("TCS")).await.unwrap().data.ltp, 10.0);
        wait_for_calls(&r.p, 2).await;
        assert!(
            r.persist
                .get("q:NSE:TCS")
                .unwrap()
                .0
                .contains("\"ltp\":10.0")
        );

        // t=35 s: another background refresh; now 30 s have passed, so it is
        // persisted.
        r.clock.advance(S(20));
        r.p.repeat_quotes(Ok(vec![quote("TCS", 12.0)]));
        assert_eq!(r.md.quote(&id("TCS")).await.unwrap().data.ltp, 11.0);
        wait_for_calls(&r.p, 3).await;
        assert!(
            r.persist
                .get("q:NSE:TCS")
                .unwrap()
                .0
                .contains("\"ltp\":12.0")
        );
    }

    #[tokio::test]
    async fn history_is_cached_and_a_stale_series_is_still_one_provider() {
        let r = rig();
        let c = Candle {
            t: 1,
            o: 1.0,
            h: 1.0,
            l: 1.0,
            c: 1.0,
            v: 1.0,
        };
        let now = Utc::now();
        let series = Sourced::new(
            CandleSeries {
                id: id("TCS"),
                interval: Interval::D1,
                candles: vec![c],
            },
            Provenance::new(ProviderId::Nse, FeedKind::Delayed, now, now),
        );
        r.p.repeat_history(Ok(series));
        let req = HistoryRequest {
            id: id("TCS"),
            interval: Interval::D1,
            range: HistoryRange::M1,
        };
        r.md.history(&req).await.unwrap();
        r.md.history(&req).await.unwrap();
        assert_eq!(r.p.history_calls.load(Ordering::SeqCst), 1);
        r.p.repeat_history(Err(DataError::Timeout));
        r.clock.advance(Duration::from_secs(48 * 3600));
        let stale = r.md.history(&req).await.unwrap();
        assert!(stale.provenance.stale);
        assert_eq!(stale.provenance.provider, ProviderId::Nse);
    }

    #[test]
    fn daily_history_is_fresh_until_the_next_close() {
        // 09:00 UTC: close (10:00 UTC) is an hour away.
        let morning = Utc.with_ymd_and_hms(2026, 11, 18, 9, 0, 0).unwrap();
        assert_eq!(daily_fresh_for(morning), S(3600));
        // 11:00 UTC: tomorrow's close is 23 hours away.
        let evening = Utc.with_ymd_and_hms(2026, 11, 18, 11, 0, 0).unwrap();
        assert_eq!(daily_fresh_for(evening), S(23 * 3600));
        // Just before close: floored at a minute.
        let edge = Utc.with_ymd_and_hms(2026, 11, 18, 9, 59, 50).unwrap();
        assert_eq!(daily_fresh_for(edge), S(60));
    }

    #[tokio::test]
    async fn the_sqlite_store_works_as_the_persistent_tier() {
        let store = Arc::new(nivesh_store::Store::open_in_memory().unwrap());
        let p = Arc::new(MockProvider::new(ProviderId::Nse, Capabilities::QUOTE));
        p.repeat_quotes(Ok(vec![quote("TCS", 10.0)]));
        let md = MarketData::builder()
            .provider(p)
            .cache(CacheConfig {
                persistent: Some(store.clone()),
                ..CacheConfig::default()
            })
            .build();
        md.quote(&id("TCS")).await.unwrap();
        assert!(store.cache_get("q:NSE:TCS").unwrap().is_some());

        let down = Arc::new(MockProvider::new(ProviderId::Nse, Capabilities::QUOTE));
        down.repeat_quotes(Err(DataError::Blocked));
        let md2 = MarketData::builder()
            .provider(down)
            .cache(CacheConfig {
                persistent: Some(store),
                ..CacheConfig::default()
            })
            .build();
        assert!(md2.quote(&id("TCS")).await.unwrap().provenance.stale);
    }
}
