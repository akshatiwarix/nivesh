//! The `MarketData` facade: the only thing the rest of the app talks to.
//!
//! It tries providers in a per-capability priority order, falling back on
//! failure; tracks provider health with a circuit breaker; and coalesces
//! identical in-flight requests. A candle series always comes whole from one
//! provider: a fallback re-fetches the series rather than splicing two sources.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use nivesh_core::bars::{normalize, resample};
use nivesh_core::{
    CandleSeries, IndexId, IndexQuote, IndexSnapshot, InstrumentId, ProviderId, Quote, Sourced,
};
use tokio::sync::broadcast;

use crate::cache::{CacheConfig, CacheLayer};
use crate::clock::{Clock, SystemClock};
use crate::constituents::ConstituentsList;
use crate::health::{Breaker, HealthConfig, Permit, ProviderStatus};
use crate::movers::compute_movers;
use crate::news::TagIndex;
use crate::singleflight::SingleFlight;
use crate::{
    Capabilities, DataError, DataProvider, HistoryRequest, Movers, MoversRequest, NewsItem,
    NewsRequest,
};

/// Headlines kept in the cached list; requests filter and trim it.
const NEWS_KEPT: usize = 200;

/// Which providers to try, in order, for each capability. Providers named here
/// but not registered (Kite before it's connected) are skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routing {
    pub quotes: Vec<ProviderId>,
    pub history: Vec<ProviderId>,
    pub index: Vec<ProviderId>,
    pub news: Vec<ProviderId>,
}

impl Default for Routing {
    fn default() -> Self {
        use ProviderId::{Kite, Nse, Rss, Yahoo};
        Self {
            quotes: vec![Kite, Nse, Yahoo],
            history: vec![Kite, Yahoo, Nse],
            index: vec![Nse, Yahoo],
            news: vec![Rss],
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) struct Inner {
    providers: HashMap<ProviderId, Arc<dyn DataProvider>>,
    routing: Routing,
    clock: Arc<dyn Clock>,
    breakers: Mutex<HashMap<ProviderId, Breaker>>,
    forced: Mutex<HashMap<ProviderId, DataError>>,
    status_tx: broadcast::Sender<ProviderStatus>,
    cache: Option<Arc<CacheLayer>>,
    constituents: Option<Arc<ConstituentsList>>,
    tagger: Option<Arc<TagIndex>>,
    news_flight: SingleFlight<(), Sourced<Vec<NewsItem>>>,
    quotes_flight: SingleFlight<Vec<InstrumentId>, Vec<Sourced<Quote>>>,
    index_flight: SingleFlight<IndexId, Sourced<IndexSnapshot>>,
    all_indices_flight: SingleFlight<(), Vec<Sourced<IndexQuote>>>,
    history_flight: SingleFlight<HistoryRequest, Sourced<CandleSeries>>,
    movers_flight: SingleFlight<MoversRequest, Sourced<Movers>>,
}

#[derive(Clone)]
pub struct MarketData {
    pub(crate) inner: Arc<Inner>,
}

pub struct MarketDataBuilder {
    providers: Vec<Arc<dyn DataProvider>>,
    routing: Routing,
    health: HealthConfig,
    clock: Arc<dyn Clock>,
    cache: Option<CacheConfig>,
    constituents: Option<Arc<ConstituentsList>>,
    tagger: Option<Arc<TagIndex>>,
}

impl MarketDataBuilder {
    pub fn provider(mut self, provider: Arc<dyn DataProvider>) -> Self {
        self.providers.push(provider);
        self
    }

    pub fn routing(mut self, routing: Routing) -> Self {
        self.routing = routing;
        self
    }

    pub fn health(mut self, health: HealthConfig) -> Self {
        self.health = health;
        self
    }

    pub fn clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Turns on the two-tier cache (memory, then the persistent store).
    pub fn cache(mut self, config: CacheConfig) -> Self {
        self.cache = Some(config);
        self
    }

    /// Dated membership lists to use when no provider supplies an index's
    /// constituents (Yahoo has the level only; NSE's list may be blocked).
    pub fn constituents(mut self, list: Arc<ConstituentsList>) -> Self {
        self.constituents = Some(list);
        self
    }

    /// Tags headlines with the companies they mention.
    pub fn tagger(mut self, tagger: Arc<TagIndex>) -> Self {
        self.tagger = Some(tagger);
        self
    }

    pub fn build(self) -> MarketData {
        let breakers = self
            .providers
            .iter()
            .map(|p| (p.id(), Breaker::new(self.health)))
            .collect();
        let (status_tx, _) = broadcast::channel(64);
        let cache = self
            .cache
            .map(|config| Arc::new(CacheLayer::new(config, Arc::clone(&self.clock))));
        MarketData {
            inner: Arc::new(Inner {
                providers: self.providers.into_iter().map(|p| (p.id(), p)).collect(),
                routing: self.routing,
                clock: self.clock,
                breakers: Mutex::new(breakers),
                forced: Mutex::new(HashMap::new()),
                status_tx,
                cache,
                constituents: self.constituents,
                tagger: self.tagger,
                news_flight: SingleFlight::new(),
                quotes_flight: SingleFlight::new(),
                index_flight: SingleFlight::new(),
                all_indices_flight: SingleFlight::new(),
                history_flight: SingleFlight::new(),
                movers_flight: SingleFlight::new(),
            }),
        }
    }
}

/// The error to report when every provider failed: a real failure beats
/// `UnsupportedRange`, which beats `NotFound`, which beats `Unsupported`.
/// `NotFound` is only reported when no provider did anything worse.
fn pick_error(errors: Vec<DataError>) -> DataError {
    let rank = |e: &DataError| match e {
        DataError::Unsupported => 0,
        DataError::NotFound => 1,
        DataError::UnsupportedRange(_) => 2,
        _ => 3,
    };
    errors
        .into_iter()
        .reduce(|best, e| if rank(&e) > rank(&best) { e } else { best })
        .unwrap_or(DataError::Unsupported)
}

impl MarketData {
    pub fn builder() -> MarketDataBuilder {
        MarketDataBuilder {
            providers: Vec::new(),
            routing: Routing::default(),
            health: HealthConfig::default(),
            clock: Arc::new(SystemClock::new()),
            cache: None,
            constituents: None,
            tagger: None,
        }
    }

    pub(crate) fn history_order(&self) -> Vec<ProviderId> {
        self.inner.routing.history.clone()
    }

    pub(crate) fn constituent_lists(&self) -> Option<&Arc<ConstituentsList>> {
        self.inner.constituents.as_ref()
    }

    pub(crate) fn cache_layer(&self) -> Option<&Arc<CacheLayer>> {
        self.inner.cache.as_ref()
    }

    /// Health changes, for the `provider:status` event.
    pub fn subscribe_status(&self) -> broadcast::Receiver<ProviderStatus> {
        self.inner.status_tx.subscribe()
    }

    /// Current health of every registered provider.
    pub fn provider_status(&self) -> Vec<ProviderStatus> {
        let mut all: Vec<_> = lock(&self.inner.breakers)
            .iter()
            .map(|(id, b)| b.status(*id))
            .collect();
        all.sort_by_key(|s| s.provider);
        all
    }

    /// Test hook: while set, every call to `provider` fails with `error`
    /// without reaching the network (and counts against its health). Pass
    /// `None` to clear. This is how "NSE is blocked" is simulated.
    pub fn force_failure(&self, provider: ProviderId, error: Option<DataError>) {
        let mut forced = lock(&self.inner.forced);
        match error {
            Some(e) => forced.insert(provider, e),
            None => forced.remove(&provider),
        };
    }

    /// Runs `call` against `order`'s providers until one succeeds.
    pub(crate) async fn route<T, F, Fut>(
        &self,
        need: Capabilities,
        order: &[ProviderId],
        call: F,
    ) -> Result<T, DataError>
    where
        F: Fn(Arc<dyn DataProvider>) -> Fut,
        Fut: Future<Output = Result<T, DataError>>,
    {
        let mut errors = Vec::new();
        for id in order {
            let Some(provider) = self.inner.providers.get(id) else {
                continue;
            };
            if !provider.capabilities().contains(need) {
                continue;
            }
            if self.permit(*id) == Permit::No {
                errors.push(DataError::Upstream(format!(
                    "{id} is temporarily skipped (circuit open)"
                )));
                continue;
            }
            let forced = lock(&self.inner.forced).get(id).cloned();
            let result = match forced {
                Some(e) => Err(e),
                None => call(Arc::clone(provider)).await,
            };
            match result {
                Ok(value) => {
                    self.record(*id, None);
                    return Ok(value);
                }
                Err(e) => {
                    tracing::debug!(provider = %id, error = %e, "provider failed; trying next");
                    self.record(*id, Some(&e));
                    errors.push(e);
                }
            }
        }
        Err(pick_error(errors))
    }

    fn permit(&self, id: ProviderId) -> Permit {
        let now = self.inner.clock.now();
        lock(&self.inner.breakers)
            .get_mut(&id)
            .map_or(Permit::Yes, |b| b.permit(now))
    }

    /// Updates a provider's breaker and publishes a status event on change.
    fn record(&self, id: ProviderId, error: Option<&DataError>) {
        let now = self.inner.clock.now();
        let status = {
            let mut breakers = lock(&self.inner.breakers);
            let Some(b) = breakers.get_mut(&id) else {
                return;
            };
            let before = (b.state(), b.status(id).consecutive_failures);
            match error {
                None => b.record_success(),
                Some(e) if e.counts_against_health() => b.record_failure(now, e),
                // Asked for something it doesn't have, or an unknown symbol:
                // the provider itself answered, so it isn't unhealthy.
                Some(DataError::NotFound) => b.record_success(),
                Some(_) => b.record_neutral(),
            }
            let after = b.status(id);
            ((before.0, before.1) != (after.state, after.consecutive_failures)).then_some(after)
        };
        if let Some(status) = status {
            // No receivers is fine: nobody is listening yet.
            let _ = self.inner.status_tx.send(status);
        }
    }

    /// Quotes for `ids`, in one provider call where possible. Instruments no
    /// provider knows are omitted; if none are known the result is `NotFound`.
    pub(crate) async fn fetch_quotes(
        &self,
        ids: &[InstrumentId],
    ) -> Result<Vec<Sourced<Quote>>, DataError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut key = ids.to_vec();
        key.sort();
        key.dedup();
        let this = self.clone();
        let request = key.clone();
        self.inner
            .quotes_flight
            .run(key, move || async move {
                let order = this.inner.routing.quotes.clone();
                this.route(Capabilities::QUOTE, &order, |p| {
                    let ids = request.clone();
                    async move { p.quotes(&ids).await }
                })
                .await
            })
            .await
    }

    /// Quotes for `ids`, in one provider call where possible. Instruments no
    /// provider knows are omitted; if none are known the result is `NotFound`.
    /// With a cache configured, fresh entries are served without a call.
    pub async fn quotes(&self, ids: &[InstrumentId]) -> Result<Vec<Sourced<Quote>>, DataError> {
        match &self.inner.cache {
            Some(cache) => crate::cache::quotes(self, cache, ids).await,
            None => self.fetch_quotes(ids).await,
        }
    }

    /// One index with its constituents where a provider has them.
    pub async fn index_snapshot(
        &self,
        index: IndexId,
    ) -> Result<Sourced<IndexSnapshot>, DataError> {
        match &self.inner.cache {
            Some(cache) => crate::cache::index_snapshot(self, cache, index).await,
            None => self.fetch_index_snapshot(index).await,
        }
    }

    /// Levels of every catalogued index, for the MKT dashboard.
    pub async fn all_indices(&self) -> Result<Vec<Sourced<IndexQuote>>, DataError> {
        match &self.inner.cache {
            Some(cache) => crate::cache::all_indices(self, cache).await,
            None => self.fetch_all_indices().await,
        }
    }

    /// Top gainers, losers and most active stocks of an index universe, all
    /// from one source.
    pub async fn movers(&self, req: &MoversRequest) -> Result<Sourced<Movers>, DataError> {
        match &self.inner.cache {
            Some(cache) => crate::cache::movers(self, cache, req).await,
            None => self.fetch_movers(req).await,
        }
    }

    /// NSE's own gainers/losers where it has them for this index, with the
    /// most-active list ranked from the same source's constituent volumes.
    /// Otherwise everything is ranked from the index's constituent quotes
    /// (whoever priced them, with their provenance), so the three lists never
    /// mix providers.
    pub(crate) async fn fetch_movers(
        &self,
        req: &MoversRequest,
    ) -> Result<Sourced<Movers>, DataError> {
        let this = self.clone();
        let request = *req;
        self.inner
            .movers_flight
            .run(request, move || async move {
                let order = this.inner.routing.index.clone();
                let direct = this
                    .route(Capabilities::MOVERS, &order, |p| async move {
                        p.movers(&request).await
                    })
                    .await;
                match direct {
                    Ok(mut movers) => {
                        // Same provider only: a different one would splice sources.
                        if let Ok(snap) = this.index_snapshot(request.index).await
                            && snap.provenance.provider == movers.provenance.provider
                        {
                            movers.data.most_active =
                                compute_movers(&snap.data.constituents, request.limit).most_active;
                        }
                        Ok(movers)
                    }
                    Err(direct_error) => {
                        let snap = this.index_snapshot(request.index).await.map_err(|e| {
                            // Prefer the more informative of the two failures.
                            pick_error(vec![direct_error, e])
                        })?;
                        if snap.data.constituents.is_empty() {
                            return Err(DataError::NotFound);
                        }
                        let movers = compute_movers(&snap.data.constituents, request.limit);
                        Ok(Sourced {
                            data: movers,
                            provenance: snap.provenance,
                        })
                    }
                }
            })
            .await
    }

    /// Recent headlines, newest first, optionally only those about
    /// `req.query`: a symbol the headline is tagged with, or text in its title.
    /// The whole feed set is fetched once and cached; a query only filters it.
    pub async fn news(&self, req: &NewsRequest) -> Result<Sourced<Vec<NewsItem>>, DataError> {
        let mut all = match &self.inner.cache {
            Some(cache) => crate::cache::news(self, cache).await?,
            None => self.fetch_news().await?,
        };
        if let Some(query) = req
            .query
            .as_deref()
            .map(str::trim)
            .filter(|q| !q.is_empty())
        {
            let upper = query.to_ascii_uppercase();
            all.data.retain(|item| {
                item.tags.contains(&upper) || item.title.to_ascii_uppercase().contains(&upper)
            });
        }
        all.data.truncate(req.limit);
        Ok(all)
    }

    pub(crate) async fn fetch_news(&self) -> Result<Sourced<Vec<NewsItem>>, DataError> {
        let this = self.clone();
        self.inner
            .news_flight
            .run((), move || async move {
                let order = this.inner.routing.news.clone();
                let request = NewsRequest {
                    query: None,
                    limit: NEWS_KEPT,
                };
                let mut items = this
                    .route(Capabilities::NEWS, &order, |p| {
                        let request = request.clone();
                        async move { p.news(&request).await }
                    })
                    .await?;
                if let Some(tagger) = &this.inner.tagger {
                    for item in &mut items {
                        item.tags = tagger.tag(&item.title);
                    }
                }
                Ok(crate::news::sourced(items, chrono::Utc::now()))
            })
            .await
    }

    /// A candle series, whole, from a single provider.
    pub async fn history(&self, req: &HistoryRequest) -> Result<Sourced<CandleSeries>, DataError> {
        match &self.inner.cache {
            Some(cache) => crate::cache::history(self, cache, req).await,
            None => self.fetch_history(req).await,
        }
    }

    /// A quote from `provider` and no other, uncached. For callers that must
    /// not mix sources (a chart's forming bar). Health tracking still applies.
    pub async fn quote_from(
        &self,
        provider: ProviderId,
        id: &InstrumentId,
    ) -> Result<Sourced<Quote>, DataError> {
        let ids = [id.clone()];
        self.route(Capabilities::QUOTE, &[provider], |p| {
            let ids = ids.clone();
            async move { p.quotes(&ids).await }
        })
        .await?
        .into_iter()
        .next()
        .ok_or(DataError::NotFound)
    }

    pub async fn quote(&self, id: &InstrumentId) -> Result<Sourced<Quote>, DataError> {
        self.quotes(std::slice::from_ref(id))
            .await?
            .into_iter()
            .next()
            .ok_or(DataError::NotFound)
    }

    pub(crate) async fn fetch_index_snapshot(
        &self,
        index: IndexId,
    ) -> Result<Sourced<IndexSnapshot>, DataError> {
        let this = self.clone();
        self.inner
            .index_flight
            .run(index, move || async move {
                let order = this.inner.routing.index.clone();
                let snapshot = this
                    .route(Capabilities::INDEX, &order, |p| async move {
                        p.index_snapshot(index).await
                    })
                    .await?;
                Ok(this.with_fallback_constituents(index, snapshot).await)
            })
            .await
    }

    /// When the provider that priced the index level has no constituents, fills
    /// them from the dated membership list, priced by that same provider so the
    /// snapshot still comes from one source. If it can't price them the list is
    /// left empty (the UI says so) rather than shown unpriced.
    async fn with_fallback_constituents(
        &self,
        index: IndexId,
        mut snapshot: Sourced<IndexSnapshot>,
    ) -> Sourced<IndexSnapshot> {
        let Some(list) = &self.inner.constituents else {
            return snapshot;
        };
        if !snapshot.data.constituents.is_empty() {
            return snapshot;
        }
        let Some((ids, info)) = list.members(index) else {
            return snapshot;
        };
        let provider = snapshot.provenance.provider;
        let priced = self
            .route(Capabilities::QUOTE, &[provider], |p| {
                let ids = ids.clone();
                async move { p.constituent_quotes(&ids).await }
            })
            .await;
        match priced {
            Ok(quotes) => {
                // Keep the list's order; drop anything priced by someone else.
                snapshot.data.constituents = quotes
                    .into_iter()
                    .filter(|q| q.provenance.provider == provider)
                    .map(|q| q.data)
                    .collect();
                if !snapshot.data.constituents.is_empty() {
                    snapshot.data.constituents_info = Some(info);
                }
            }
            Err(error) => {
                tracing::debug!(index = index.code(), %error, "could not price the fallback constituents");
            }
        }
        snapshot
    }

    /// Levels of every catalogued index, for the MKT dashboard. The first
    /// provider that answers supplies most rows; indices it lacks (SENSEX on
    /// NSE) are filled individually from the next provider. Each row keeps its
    /// own provenance, so a mixed table says which row came from where.
    pub(crate) async fn fetch_all_indices(&self) -> Result<Vec<Sourced<IndexQuote>>, DataError> {
        let this = self.clone();
        self.inner
            .all_indices_flight
            .run((), move || async move {
                let order = this.inner.routing.index.clone();
                let mut rows = this
                    .route(Capabilities::INDEX, &order, |p| async move { p.all_indices().await })
                    .await?;
                let missing: Vec<IndexId> = IndexId::all()
                    .filter(|i| !rows.iter().any(|r| r.data.index == *i))
                    .collect();
                for index in missing {
                    match this.index_snapshot(index).await {
                        Ok(snap) => rows.push(snap.map(|s| s.quote)),
                        Err(e) => tracing::debug!(index = index.code(), error = %e, "no provider has this index"),
                    }
                }
                rows.sort_by_key(|r| r.data.index);
                Ok(rows)
            })
            .await
    }

    /// A candle series from the first provider that can serve all of it.
    pub(crate) async fn fetch_history(
        &self,
        req: &HistoryRequest,
    ) -> Result<Sourced<CandleSeries>, DataError> {
        let this = self.clone();
        let request = req.clone();
        self.inner
            .history_flight
            .run(req.clone(), move || async move {
                let order = this.inner.routing.history.clone();
                this.route(Capabilities::HISTORY, &order, |p| {
                    let request = request.clone();
                    async move { fetch_series(p.as_ref(), &request).await }
                })
                .await
            })
            .await
    }
}

/// One provider's answer to `request`, as exactly the series that was asked
/// for. Intervals no provider offers (4h) are built from the base interval's
/// bars with session-aligned resampling; the provider's rows are cleaned of
/// nulls and duplicates first. Anything else wrong is rejected, not patched.
async fn fetch_series(
    p: &dyn DataProvider,
    request: &HistoryRequest,
) -> Result<Sourced<CandleSeries>, DataError> {
    let base = request.interval.base();
    let asked = HistoryRequest {
        interval: base,
        ..request.clone()
    };
    let mut series = p.history(&asked).await?;
    if series.data.id != request.id || series.data.interval != base {
        return Err(DataError::Parse(
            "provider returned a different series".into(),
        ));
    }
    let mut candles = normalize(std::mem::take(&mut series.data.candles));
    if base != request.interval {
        candles = resample(&candles, request.interval);
    }
    if candles.is_empty() {
        return Err(DataError::NotFound);
    }
    series.data.candles = candles;
    series.data.interval = request.interval;
    Ok(series)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use chrono::Utc;
    use nivesh_core::{Candle, FeedKind, Interval, Provenance};

    use super::*;
    use crate::clock::ManualClock;
    use crate::health::HealthState;
    use crate::mock::MockProvider;
    use crate::news::TagIndex;
    use crate::{HistoryRange, Mover};

    fn id(s: &str) -> InstrumentId {
        InstrumentId::parse(s).unwrap()
    }

    fn prov(p: ProviderId) -> Provenance {
        let now = Utc::now();
        Provenance::new(p, FeedKind::Delayed, now, now)
    }

    fn quote(p: ProviderId, sym: &str) -> Sourced<Quote> {
        Sourced::new(
            Quote {
                id: id(sym),
                name: None,
                ltp: 100.0,
                prev_close: 99.0,
                open: None,
                high: None,
                low: None,
                volume: None,
                week52_high: None,
                week52_low: None,
                upper_band: None,
                lower_band: None,
            },
            prov(p),
        )
    }

    fn index_quote(p: ProviderId, index: IndexId) -> Sourced<IndexQuote> {
        Sourced::new(
            IndexQuote {
                index,
                last: 1.0,
                prev_close: 1.0,
                open: None,
                high: None,
                low: None,
                week52_high: None,
                week52_low: None,
                advances: None,
                declines: None,
            },
            prov(p),
        )
    }

    fn series(p: ProviderId, sym: &str, interval: Interval) -> Sourced<CandleSeries> {
        let c = Candle {
            t: 1,
            o: 1.0,
            h: 1.0,
            l: 1.0,
            c: 1.0,
            v: 1.0,
        };
        Sourced::new(
            CandleSeries {
                id: id(sym),
                interval,
                candles: vec![c],
            },
            prov(p),
        )
    }

    fn mock(p: ProviderId) -> Arc<MockProvider> {
        Arc::new(MockProvider::new(
            p,
            Capabilities::QUOTE
                | Capabilities::HISTORY
                | Capabilities::INDEX
                | Capabilities::MOVERS,
        ))
    }

    struct Rig {
        md: MarketData,
        nse: Arc<MockProvider>,
        yahoo: Arc<MockProvider>,
        clock: Arc<ManualClock>,
    }

    fn rig() -> Rig {
        let (nse, yahoo) = (mock(ProviderId::Nse), mock(ProviderId::Yahoo));
        let clock = Arc::new(ManualClock::new());
        let md = MarketData::builder()
            .provider(nse.clone())
            .provider(yahoo.clone())
            .clock(clock.clone())
            .health(HealthConfig {
                failure_threshold: 3,
                cooldown: Duration::from_secs(30),
                ..HealthConfig::default()
            })
            .build();
        Rig {
            md,
            nse,
            yahoo,
            clock,
        }
    }

    #[tokio::test]
    async fn uses_the_first_provider_when_it_works() {
        let r = rig();
        r.nse.repeat_quotes(Ok(vec![quote(ProviderId::Nse, "TCS")]));
        let q = r.md.quote(&id("TCS")).await.unwrap();
        assert_eq!(q.provenance.provider, ProviderId::Nse);
        assert_eq!(MockProvider::calls(&r.yahoo.quote_calls), 0);
    }

    #[tokio::test]
    async fn falls_back_on_every_failure_kind() {
        let errors = [
            DataError::Unsupported,
            DataError::UnsupportedRange("x".into()),
            DataError::RateLimited { retry_after: None },
            DataError::Blocked,
            DataError::Unauthorized,
            DataError::NotFound,
            DataError::Timeout,
            DataError::Upstream("boom".into()),
            DataError::Parse("bad".into()),
        ];
        for e in errors {
            let r = rig();
            r.nse.repeat_quotes(Err(e.clone()));
            r.yahoo
                .repeat_quotes(Ok(vec![quote(ProviderId::Yahoo, "TCS")]));
            let q = r.md.quote(&id("TCS")).await.unwrap();
            assert_eq!(q.provenance.provider, ProviderId::Yahoo, "after {e:?}");
        }
    }

    #[tokio::test]
    async fn not_found_is_only_reported_when_every_provider_says_so() {
        let r = rig();
        r.nse.repeat_quotes(Err(DataError::NotFound));
        r.yahoo.repeat_quotes(Err(DataError::NotFound));
        assert_eq!(
            r.md.quote(&id("ZZZ")).await.unwrap_err(),
            DataError::NotFound
        );

        let r = rig();
        r.nse.repeat_quotes(Err(DataError::NotFound));
        r.yahoo.repeat_quotes(Err(DataError::Timeout));
        assert_eq!(
            r.md.quote(&id("ZZZ")).await.unwrap_err(),
            DataError::Timeout
        );

        let r = rig();
        r.nse.repeat_quotes(Err(DataError::Blocked));
        r.yahoo.repeat_quotes(Err(DataError::NotFound));
        assert_eq!(
            r.md.quote(&id("ZZZ")).await.unwrap_err(),
            DataError::Blocked
        );
    }

    #[tokio::test]
    async fn providers_without_the_capability_are_never_called() {
        let nse = Arc::new(MockProvider::new(ProviderId::Nse, Capabilities::INDEX));
        let yahoo = mock(ProviderId::Yahoo);
        yahoo.repeat_quotes(Ok(vec![quote(ProviderId::Yahoo, "TCS")]));
        let md = MarketData::builder()
            .provider(nse.clone())
            .provider(yahoo)
            .build();
        md.quote(&id("TCS")).await.unwrap();
        assert_eq!(MockProvider::calls(&nse.quote_calls), 0);
    }

    #[tokio::test]
    async fn no_providers_at_all_is_unsupported() {
        let md = MarketData::builder().build();
        assert_eq!(
            md.quote(&id("TCS")).await.unwrap_err(),
            DataError::Unsupported
        );
        assert_eq!(md.quotes(&[]).await.unwrap(), Vec::new());
    }

    #[tokio::test]
    async fn breaker_opens_skips_the_provider_then_probes_after_cooldown() {
        let r = rig();
        r.nse.repeat_quotes(Err(DataError::Blocked));
        r.yahoo
            .repeat_quotes(Ok(vec![quote(ProviderId::Yahoo, "TCS")]));
        for _ in 0..3 {
            r.md.quote(&id("TCS")).await.unwrap();
        }
        assert_eq!(MockProvider::calls(&r.nse.quote_calls), 3);
        let nse_status =
            r.md.provider_status()
                .into_iter()
                .find(|s| s.provider == ProviderId::Nse)
                .unwrap();
        assert_eq!(nse_status.state, HealthState::Down);

        // Breaker open: NSE is no longer called.
        for _ in 0..5 {
            r.md.quote(&id("TCS")).await.unwrap();
        }
        assert_eq!(MockProvider::calls(&r.nse.quote_calls), 3);

        // After the cool-down exactly one probe goes through. It fails again.
        r.clock.advance(Duration::from_secs(31));
        r.md.quote(&id("TCS")).await.unwrap();
        assert_eq!(MockProvider::calls(&r.nse.quote_calls), 4);
        r.md.quote(&id("TCS")).await.unwrap();
        assert_eq!(MockProvider::calls(&r.nse.quote_calls), 4);

        // The failed probe doubled the cool-down (60 s): 31 s later is too soon.
        r.clock.advance(Duration::from_secs(31));
        r.md.quote(&id("TCS")).await.unwrap();
        assert_eq!(MockProvider::calls(&r.nse.quote_calls), 4);

        // NSE recovers; the next probe succeeds and closes the breaker.
        r.nse.repeat_quotes(Ok(vec![quote(ProviderId::Nse, "TCS")]));
        r.clock.advance(Duration::from_secs(31));
        let q = r.md.quote(&id("TCS")).await.unwrap();
        assert_eq!(q.provenance.provider, ProviderId::Nse);
        let nse_status =
            r.md.provider_status()
                .into_iter()
                .find(|s| s.provider == ProviderId::Nse)
                .unwrap();
        assert_eq!(nse_status.state, HealthState::Healthy);
    }

    #[tokio::test]
    async fn unknown_symbols_do_not_trip_the_breaker() {
        let r = rig();
        r.nse.repeat_quotes(Err(DataError::NotFound));
        r.yahoo.repeat_quotes(Err(DataError::NotFound));
        for _ in 0..10 {
            let _ = r.md.quote(&id("ZZZ")).await;
        }
        assert!(
            r.md.provider_status()
                .iter()
                .all(|s| s.state == HealthState::Healthy)
        );
        assert_eq!(MockProvider::calls(&r.nse.quote_calls), 10);
    }

    #[tokio::test]
    async fn health_changes_are_published() {
        let r = rig();
        let mut rx = r.md.subscribe_status();
        r.nse.repeat_quotes(Err(DataError::Blocked));
        r.yahoo
            .repeat_quotes(Ok(vec![quote(ProviderId::Yahoo, "TCS")]));
        for _ in 0..3 {
            r.md.quote(&id("TCS")).await.unwrap();
        }
        let mut seen = Vec::new();
        while let Ok(s) = rx.try_recv() {
            seen.push((s.provider, s.state));
        }
        assert_eq!(
            seen,
            [
                (ProviderId::Nse, HealthState::Degraded),
                (ProviderId::Nse, HealthState::Degraded), // 2 failures: same state, new count
                (ProviderId::Nse, HealthState::Down),
            ]
        );
    }

    #[tokio::test]
    async fn concurrent_identical_requests_make_one_upstream_call() {
        let nse = Arc::new(
            MockProvider::new(ProviderId::Nse, Capabilities::QUOTE)
                .with_delay(Duration::from_millis(40)),
        );
        nse.repeat_quotes(Ok(vec![quote(ProviderId::Nse, "TCS")]));
        let md = MarketData::builder().provider(nse.clone()).build();
        let tcs = id("TCS");
        let results = futures::future::join_all((0..8).map(|_| md.quote(&tcs))).await;
        assert!(results.iter().all(Result::is_ok));
        assert_eq!(nse.quote_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn quote_batches_with_the_same_ids_in_any_order_coalesce() {
        let nse = Arc::new(
            MockProvider::new(ProviderId::Nse, Capabilities::QUOTE)
                .with_delay(Duration::from_millis(40)),
        );
        nse.repeat_quotes(Ok(vec![
            quote(ProviderId::Nse, "TCS"),
            quote(ProviderId::Nse, "INFY"),
        ]));
        let md = MarketData::builder().provider(nse.clone()).build();
        let forward = [id("TCS"), id("INFY")];
        let reverse = [id("INFY"), id("TCS")];
        let (a, b) = tokio::join!(md.quotes(&forward), md.quotes(&reverse));
        assert!(a.is_ok() && b.is_ok());
        assert_eq!(nse.quote_calls.load(Ordering::SeqCst), 1);
    }

    // ---- fallback constituents ----

    fn level_only(p: ProviderId, index: IndexId) -> Sourced<IndexSnapshot> {
        index_quote(p, index).map(|quote| IndexSnapshot {
            quote,
            constituents: vec![],
            constituents_info: None,
        })
    }

    fn with_list(r: &Rig) -> MarketData {
        MarketData::builder()
            .provider(r.nse.clone())
            .provider(r.yahoo.clone())
            .constituents(Arc::new(ConstituentsList::bundled().unwrap()))
            .build()
    }

    /// Yahoo prices every symbol it is asked about.
    fn yahoo_prices_everything(r: &Rig, symbols: &[&str]) {
        r.yahoo.repeat_quotes(Ok(symbols
            .iter()
            .map(|s| quote(ProviderId::Yahoo, s))
            .collect()));
    }

    #[tokio::test]
    async fn blocked_nse_gets_dated_constituents_priced_by_yahoo() {
        let r = rig();
        let md = with_list(&r);
        r.nse.repeat_index(Err(DataError::Blocked));
        r.yahoo
            .repeat_index(Ok(level_only(ProviderId::Yahoo, IndexId::NiftyBank)));
        yahoo_prices_everything(&r, &["HDFCBANK", "ICICIBANK", "SBIN"]);

        let snap = md.index_snapshot(IndexId::NiftyBank).await.unwrap();
        assert_eq!(snap.provenance.provider, ProviderId::Yahoo);
        let symbols: Vec<_> = snap
            .data
            .constituents
            .iter()
            .map(|q| q.id.symbol())
            .collect();
        assert_eq!(symbols, ["HDFCBANK", "ICICIBANK", "SBIN"]);
        let info = snap.data.constituents_info.unwrap();
        let bundled = crate::constituents::ConstituentSet::bundled().unwrap();
        assert_eq!(info.as_of, bundled.as_of);
        assert_eq!(info.source, bundled.source);
        // The ask was for the whole bundled NIFTY BANK list, in its order.
        let asked = r.yahoo.quote_requests().remove(0);
        let wanted: Vec<_> = bundled.lists["BANKNIFTY"]
            .iter()
            .map(String::as_str)
            .collect();
        assert_eq!(asked.iter().map(|i| i.symbol()).collect::<Vec<_>>(), wanted);
        // NSE was asked for the level only, never for the fallback prices.
        assert_eq!(MockProvider::calls(&r.nse.quote_calls), 0);
    }

    #[tokio::test]
    async fn live_constituents_are_left_alone() {
        let r = rig();
        let md = with_list(&r);
        let live = Sourced::new(
            IndexSnapshot {
                quote: index_quote(ProviderId::Nse, IndexId::Nifty50).data,
                constituents: vec![quote(ProviderId::Nse, "TCS").data],
                constituents_info: None,
            },
            prov(ProviderId::Nse),
        );
        r.nse.repeat_index(Ok(live));
        let snap = md.index_snapshot(IndexId::Nifty50).await.unwrap();
        assert_eq!(snap.data.constituents.len(), 1);
        assert!(
            snap.data.constituents_info.is_none(),
            "no as-of label on live data"
        );
        assert_eq!(MockProvider::calls(&r.yahoo.quote_calls), 0);
    }

    #[tokio::test]
    async fn fallback_prices_only_come_from_the_provider_that_priced_the_level() {
        let r = rig();
        let md = with_list(&r);
        r.nse.repeat_index(Err(DataError::Blocked));
        r.yahoo
            .repeat_index(Ok(level_only(ProviderId::Yahoo, IndexId::NiftyIt)));
        // Yahoo's quote call is answered, but with rows stamped by NSE.
        r.yahoo
            .repeat_quotes(Ok(vec![quote(ProviderId::Nse, "TCS")]));
        let snap = md.index_snapshot(IndexId::NiftyIt).await.unwrap();
        assert!(snap.data.constituents.is_empty());
        assert!(snap.data.constituents_info.is_none());
    }

    #[tokio::test]
    async fn unpriceable_fallback_leaves_the_level_alone_without_a_label() {
        let r = rig();
        let md = with_list(&r);
        r.nse.repeat_index(Err(DataError::Blocked));
        r.yahoo
            .repeat_index(Ok(level_only(ProviderId::Yahoo, IndexId::NiftyIt)));
        r.yahoo.repeat_quotes(Err(DataError::Timeout));
        let snap = md.index_snapshot(IndexId::NiftyIt).await.unwrap();
        assert!(snap.data.constituents.is_empty());
        assert!(snap.data.constituents_info.is_none());
    }

    #[tokio::test]
    async fn sensex_and_unconfigured_chains_have_no_fallback() {
        let r = rig();
        let md = with_list(&r);
        r.nse.repeat_index(Err(DataError::Unsupported));
        r.yahoo
            .repeat_index(Ok(level_only(ProviderId::Yahoo, IndexId::Sensex)));
        let snap = md.index_snapshot(IndexId::Sensex).await.unwrap();
        assert!(snap.data.constituents.is_empty());
        assert_eq!(MockProvider::calls(&r.yahoo.quote_calls), 0);

        // Without a list the chain behaves as before.
        let r = rig();
        r.nse.repeat_index(Err(DataError::Blocked));
        r.yahoo
            .repeat_index(Ok(level_only(ProviderId::Yahoo, IndexId::Nifty50)));
        let snap = r.md.index_snapshot(IndexId::Nifty50).await.unwrap();
        assert!(snap.data.constituents.is_empty());
        assert_eq!(MockProvider::calls(&r.yahoo.quote_calls), 0);
    }

    fn snapshot_with(p: ProviderId, rows: &[(&str, f64, Option<u64>)]) -> Sourced<IndexSnapshot> {
        let mut snap = level_only(p, IndexId::Nifty50);
        snap.data.constituents = rows
            .iter()
            .map(|(sym, ltp, volume)| Quote {
                ltp: *ltp,
                volume: *volume,
                ..quote(p, sym).data
            })
            .collect();
        snap
    }

    fn nse_movers(gainer: &str) -> Sourced<Movers> {
        let m = Mover {
            quote: Quote {
                ltp: 110.0,
                ..quote(ProviderId::Nse, gainer).data
            },
        };
        Sourced::new(
            Movers {
                gainers: vec![m],
                ..Movers::default()
            },
            prov(ProviderId::Nse),
        )
    }

    const REQ: MoversRequest = MoversRequest {
        index: IndexId::Nifty50,
        limit: 5,
    };

    #[tokio::test]
    async fn movers_use_nses_lists_and_rank_most_active_from_the_same_source() {
        let r = rig();
        r.nse.repeat_movers(Ok(nse_movers("TATASTEEL")));
        r.nse.repeat_index(Ok(snapshot_with(
            ProviderId::Nse,
            &[("A", 101.0, Some(10)), ("B", 102.0, Some(99))],
        )));
        let m = r.md.movers(&REQ).await.unwrap();
        assert_eq!(m.provenance.provider, ProviderId::Nse);
        assert_eq!(m.data.gainers[0].quote.id.symbol(), "TATASTEEL");
        assert_eq!(m.data.most_active[0].quote.id.symbol(), "B");
    }

    #[tokio::test]
    async fn movers_never_splice_a_second_source_into_nses_lists() {
        let r = rig();
        r.nse.repeat_movers(Ok(nse_movers("TATASTEEL")));
        // NSE's index snapshot fails, so only Yahoo could price the constituents.
        r.nse.repeat_index(Err(DataError::Blocked));
        r.yahoo.repeat_index(Ok(snapshot_with(
            ProviderId::Yahoo,
            &[("A", 101.0, Some(10)), ("B", 102.0, Some(99))],
        )));
        let m = r.md.movers(&REQ).await.unwrap();
        assert_eq!(m.provenance.provider, ProviderId::Nse);
        assert!(m.data.most_active.is_empty());
    }

    #[tokio::test]
    async fn movers_fall_back_to_ranking_the_constituents_with_their_provenance() {
        let r = rig();
        r.nse.repeat_movers(Err(DataError::Blocked));
        r.nse.repeat_index(Err(DataError::Blocked));
        r.yahoo.repeat_index(Ok(snapshot_with(
            ProviderId::Yahoo,
            &[("UP", 105.0, None), ("DOWN", 95.0, None)],
        )));
        let m = r.md.movers(&REQ).await.unwrap();
        assert_eq!(m.provenance.provider, ProviderId::Yahoo);
        assert_eq!(m.data.gainers[0].quote.id.symbol(), "UP");
        assert_eq!(m.data.losers[0].quote.id.symbol(), "DOWN");
        assert!(m.data.most_active.is_empty(), "no volume from this source");
    }

    #[tokio::test]
    async fn an_index_nse_publishes_no_movers_for_is_ranked_from_constituents() {
        let r = rig();
        r.nse.repeat_movers(Err(DataError::Unsupported));
        r.nse.repeat_index(Ok(snapshot_with(
            ProviderId::Nse,
            &[("A", 103.0, Some(5)), ("B", 98.0, Some(9))],
        )));
        let m = r.md.movers(&REQ).await.unwrap();
        assert_eq!(m.provenance.provider, ProviderId::Nse);
        assert_eq!(m.data.gainers[0].quote.id.symbol(), "A");
        assert_eq!(m.data.most_active[0].quote.id.symbol(), "B");
    }

    #[tokio::test]
    async fn movers_fail_when_nothing_can_be_priced() {
        let r = rig();
        r.nse.repeat_movers(Err(DataError::Blocked));
        r.nse.repeat_index(Err(DataError::Blocked));
        r.yahoo.repeat_index(Err(DataError::Timeout));
        assert!(r.md.movers(&REQ).await.is_err());
    }

    fn headline(title: &str, minute: u32) -> NewsItem {
        use chrono::TimeZone as _;
        NewsItem {
            title: title.into(),
            source: "ET".into(),
            url: format!("https://example.com/{minute}"),
            published: Utc.with_ymd_and_hms(2026, 11, 18, 9, minute, 0).unwrap(),
            tags: vec![],
        }
    }

    fn news_market(rss: &Arc<MockProvider>) -> MarketData {
        let entries = vec![
            (
                "RELIANCE".to_owned(),
                "Reliance Industries Limited".to_owned(),
                0,
            ),
            (
                "TCS".to_owned(),
                "Tata Consultancy Services Limited".to_owned(),
                0,
            ),
        ];
        MarketData::builder()
            .provider(rss.clone())
            .tagger(Arc::new(TagIndex::new(&entries)))
            .cache(crate::CacheConfig::default())
            .build()
    }

    fn rss_mock() -> Arc<MockProvider> {
        let rss = Arc::new(MockProvider::new(ProviderId::Rss, Capabilities::NEWS));
        rss.repeat_news(Ok(vec![
            headline("Reliance Industries hits record high", 30),
            headline("Sensex ends higher", 20),
            headline("TCS bags big contract", 10),
        ]));
        rss
    }

    #[tokio::test]
    async fn news_is_tagged_labelled_delayed_and_fetched_once() {
        let rss = rss_mock();
        let md = news_market(&rss);
        let req = NewsRequest {
            query: None,
            limit: 10,
        };
        let news = md.news(&req).await.unwrap();
        assert_eq!(news.provenance.provider, ProviderId::Rss);
        assert_eq!(news.provenance.feed, FeedKind::Delayed);
        assert_eq!(news.data[0].tags, ["RELIANCE"]);
        assert_eq!(news.data[2].tags, ["TCS"]);
        assert!(news.data[1].tags.is_empty());
        // As of the newest headline.
        assert_eq!(news.provenance.as_of, news.data[0].published);
        md.news(&NewsRequest {
            query: Some("tcs".into()),
            limit: 10,
        })
        .await
        .unwrap();
        assert_eq!(
            MockProvider::calls(&rss.news_calls),
            1,
            "queries filter the cached list"
        );
    }

    #[tokio::test]
    async fn news_can_be_filtered_by_symbol_tag_or_title_text_and_limited() {
        let md = news_market(&rss_mock());
        let ask = |q: Option<&str>, limit| NewsRequest {
            query: q.map(str::to_owned),
            limit,
        };
        let titles =
            |n: Sourced<Vec<NewsItem>>| n.data.into_iter().map(|i| i.title).collect::<Vec<_>>();
        assert_eq!(
            titles(md.news(&ask(Some("RELIANCE"), 10)).await.unwrap()),
            ["Reliance Industries hits record high"]
        );
        assert_eq!(
            titles(md.news(&ask(Some("sensex"), 10)).await.unwrap()),
            ["Sensex ends higher"]
        );
        assert!(
            md.news(&ask(Some("nothing"), 10))
                .await
                .unwrap()
                .data
                .is_empty()
        );
        assert_eq!(md.news(&ask(None, 2)).await.unwrap().data.len(), 2);
    }

    #[tokio::test]
    async fn news_errors_when_the_feeds_are_unreachable_and_nothing_is_cached() {
        let rss = Arc::new(MockProvider::new(ProviderId::Rss, Capabilities::NEWS));
        rss.repeat_news(Err(DataError::Timeout));
        let err = news_market(&rss)
            .news(&NewsRequest {
                query: None,
                limit: 5,
            })
            .await
            .unwrap_err();
        assert_eq!(err, DataError::Timeout);
    }

    #[tokio::test]
    async fn history_comes_whole_from_one_provider() {
        let r = rig();
        let req = HistoryRequest {
            id: id("TCS"),
            interval: Interval::D1,
            range: HistoryRange::M1,
        };
        r.nse.repeat_history(Err(DataError::Unsupported));
        r.yahoo
            .repeat_history(Ok(series(ProviderId::Yahoo, "TCS", Interval::D1)));
        let h = r.md.history(&req).await.unwrap();
        assert_eq!(h.provenance.provider, ProviderId::Yahoo);
    }

    #[tokio::test]
    async fn four_hour_bars_are_built_from_one_providers_hourly_bars() {
        let r = rig();
        let req = HistoryRequest {
            id: id("TCS"),
            interval: Interval::H4,
            range: HistoryRange::M1,
        };
        // 2026-11-16 09:15, 10:15, 13:15 and 14:15 IST, hourly.
        let at = |h: i64, m: i64| (1_794_767_400 + h * 3600 + m * 60) * 1000;
        let hourly = [at(9, 15), at(10, 15), at(13, 15), at(14, 15)].map(|t| Candle {
            t,
            o: 10.0,
            h: 12.0,
            l: 9.0,
            c: 11.0,
            v: 5.0,
        });
        let mut s = series(ProviderId::Yahoo, "TCS", Interval::H1);
        s.data.candles = hourly.to_vec();
        r.nse.repeat_history(Err(DataError::Unsupported));
        r.yahoo.repeat_history(Ok(s));
        let h = r.md.history(&req).await.unwrap();
        assert_eq!(h.data.interval, Interval::H4);
        assert_eq!(h.provenance.provider, ProviderId::Yahoo);
        assert_eq!(
            h.data
                .candles
                .iter()
                .map(|c| (c.t, c.v))
                .collect::<Vec<_>>(),
            [(at(9, 15), 10.0), (at(13, 15), 10.0)]
        );
    }

    #[tokio::test]
    async fn a_mismatched_series_is_rejected_and_refetched_whole_elsewhere() {
        let r = rig();
        let req = HistoryRequest {
            id: id("TCS"),
            interval: Interval::D1,
            range: HistoryRange::M1,
        };
        // The first provider answers with the wrong interval: not accepted.
        let md = MarketData::builder()
            .provider(r.yahoo.clone())
            .provider(r.nse.clone())
            .routing(Routing {
                history: vec![ProviderId::Yahoo, ProviderId::Nse],
                ..Routing::default()
            })
            .build();
        r.yahoo
            .repeat_history(Ok(series(ProviderId::Yahoo, "TCS", Interval::M5)));
        r.nse
            .repeat_history(Ok(series(ProviderId::Nse, "TCS", Interval::D1)));
        let h = md.history(&req).await.unwrap();
        assert_eq!(h.provenance.provider, ProviderId::Nse);
        assert_eq!(h.data.interval, Interval::D1);
    }

    #[tokio::test]
    async fn range_limits_surface_when_no_provider_can_serve() {
        let r = rig();
        let req = HistoryRequest {
            id: id("TCS"),
            interval: Interval::M1,
            range: HistoryRange::M3,
        };
        r.nse.repeat_history(Err(DataError::Unsupported));
        r.yahoo.repeat_history(Err(DataError::UnsupportedRange(
            "1-minute bars cover 7 days".into(),
        )));
        assert!(matches!(
            r.md.history(&req).await.unwrap_err(),
            DataError::UnsupportedRange(_)
        ));
    }

    #[tokio::test]
    async fn all_indices_fills_what_the_first_provider_lacks_keeping_provenance() {
        let r = rig();
        let nse_rows: Vec<_> = IndexId::all()
            .filter(|i| *i != IndexId::Sensex)
            .map(|i| index_quote(ProviderId::Nse, i))
            .collect();
        r.nse.repeat_all_indices(Ok(nse_rows));
        r.nse.repeat_index(Err(DataError::Unsupported));
        r.yahoo.repeat_index(Ok(Sourced::new(
            IndexSnapshot {
                quote: index_quote(ProviderId::Yahoo, IndexId::Sensex).data,
                constituents: vec![],
                constituents_info: None,
            },
            prov(ProviderId::Yahoo),
        )));
        let rows = r.md.all_indices().await.unwrap();
        assert_eq!(rows.len(), 13);
        let by = |i: IndexId| {
            rows.iter()
                .find(|r| r.data.index == i)
                .unwrap()
                .provenance
                .provider
        };
        assert_eq!(by(IndexId::Nifty50), ProviderId::Nse);
        assert_eq!(by(IndexId::Sensex), ProviderId::Yahoo);
        assert!(
            rows.windows(2).all(|w| w[0].data.index < w[1].data.index),
            "dashboard order"
        );
    }

    #[tokio::test]
    async fn all_indices_falls_back_wholesale_when_nse_is_down() {
        let r = rig();
        r.nse.repeat_all_indices(Err(DataError::Blocked));
        r.yahoo.repeat_all_indices(Ok(IndexId::all()
            .map(|i| index_quote(ProviderId::Yahoo, i))
            .collect()));
        let rows = r.md.all_indices().await.unwrap();
        assert_eq!(rows.len(), 13);
        assert!(
            rows.iter()
                .all(|r| r.provenance.provider == ProviderId::Yahoo)
        );
        assert_eq!(
            MockProvider::calls(&r.yahoo.index_calls),
            0,
            "nothing was missing, nothing to fill"
        );
    }

    #[tokio::test]
    async fn forcing_nse_to_fail_falls_back_to_yahoo_without_calling_nse() {
        let r = rig();
        r.nse.repeat_quotes(Ok(vec![quote(ProviderId::Nse, "TCS")]));
        r.yahoo
            .repeat_quotes(Ok(vec![quote(ProviderId::Yahoo, "TCS")]));
        r.md.force_failure(ProviderId::Nse, Some(DataError::Blocked));
        let q = r.md.quote(&id("TCS")).await.unwrap();
        assert_eq!(q.provenance.provider, ProviderId::Yahoo);
        assert_eq!(MockProvider::calls(&r.nse.quote_calls), 0);
        r.md.force_failure(ProviderId::Nse, None);
        assert_eq!(
            r.md.quote(&id("TCS")).await.unwrap().provenance.provider,
            ProviderId::Nse
        );
    }

    #[test]
    fn default_routing_matches_the_plan() {
        let r = Routing::default();
        assert_eq!(
            r.quotes,
            [ProviderId::Kite, ProviderId::Nse, ProviderId::Yahoo]
        );
        assert_eq!(
            r.history,
            [ProviderId::Kite, ProviderId::Yahoo, ProviderId::Nse]
        );
        assert_eq!(r.index, [ProviderId::Nse, ProviderId::Yahoo]);
    }
}
