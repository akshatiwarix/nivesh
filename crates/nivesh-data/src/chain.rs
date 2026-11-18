//! The `MarketData` facade: the only thing the rest of the app talks to.
//!
//! It tries providers in a per-capability priority order, falling back on
//! failure; tracks provider health with a circuit breaker; and coalesces
//! identical in-flight requests. A candle series always comes whole from one
//! provider: a fallback re-fetches the series rather than splicing two sources.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use nivesh_core::{
    CandleSeries, IndexId, IndexQuote, IndexSnapshot, InstrumentId, ProviderId, Quote, Sourced,
};
use tokio::sync::broadcast;

use crate::clock::{Clock, SystemClock};
use crate::health::{Breaker, HealthConfig, Permit, ProviderStatus};
use crate::singleflight::SingleFlight;
use crate::{Capabilities, DataError, DataProvider, HistoryRequest};

/// Which providers to try, in order, for each capability. Providers named here
/// but not registered (Kite before it's connected) are skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routing {
    pub quotes: Vec<ProviderId>,
    pub history: Vec<ProviderId>,
    pub index: Vec<ProviderId>,
}

impl Default for Routing {
    fn default() -> Self {
        use ProviderId::{Kite, Nse, Yahoo};
        Self {
            quotes: vec![Kite, Nse, Yahoo],
            history: vec![Kite, Yahoo, Nse],
            index: vec![Nse, Yahoo],
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
    quotes_flight: SingleFlight<Vec<InstrumentId>, Vec<Sourced<Quote>>>,
    index_flight: SingleFlight<IndexId, Sourced<IndexSnapshot>>,
    all_indices_flight: SingleFlight<(), Vec<Sourced<IndexQuote>>>,
    history_flight: SingleFlight<HistoryRequest, Sourced<CandleSeries>>,
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

    pub fn build(self) -> MarketData {
        let breakers = self
            .providers
            .iter()
            .map(|p| (p.id(), Breaker::new(self.health)))
            .collect();
        let (status_tx, _) = broadcast::channel(64);
        MarketData {
            inner: Arc::new(Inner {
                providers: self.providers.into_iter().map(|p| (p.id(), p)).collect(),
                routing: self.routing,
                clock: self.clock,
                breakers: Mutex::new(breakers),
                forced: Mutex::new(HashMap::new()),
                status_tx,
                quotes_flight: SingleFlight::new(),
                index_flight: SingleFlight::new(),
                all_indices_flight: SingleFlight::new(),
                history_flight: SingleFlight::new(),
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
        }
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
    async fn route<T, F, Fut>(
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
    pub async fn quotes(&self, ids: &[InstrumentId]) -> Result<Vec<Sourced<Quote>>, DataError> {
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

    pub async fn quote(&self, id: &InstrumentId) -> Result<Sourced<Quote>, DataError> {
        self.quotes(std::slice::from_ref(id))
            .await?
            .into_iter()
            .next()
            .ok_or(DataError::NotFound)
    }

    pub async fn index_snapshot(
        &self,
        index: IndexId,
    ) -> Result<Sourced<IndexSnapshot>, DataError> {
        let this = self.clone();
        self.inner
            .index_flight
            .run(index, move || async move {
                let order = this.inner.routing.index.clone();
                this.route(Capabilities::INDEX, &order, |p| async move {
                    p.index_snapshot(index).await
                })
                .await
            })
            .await
    }

    /// Levels of every catalogued index, for the MKT dashboard. The first
    /// provider that answers supplies most rows; indices it lacks (SENSEX on
    /// NSE) are filled individually from the next provider. Each row keeps its
    /// own provenance, so a mixed table says which row came from where.
    pub async fn all_indices(&self) -> Result<Vec<Sourced<IndexQuote>>, DataError> {
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
    pub async fn history(&self, req: &HistoryRequest) -> Result<Sourced<CandleSeries>, DataError> {
        let this = self.clone();
        let request = req.clone();
        self.inner
            .history_flight
            .run(req.clone(), move || async move {
                let order = this.inner.routing.history.clone();
                this.route(Capabilities::HISTORY, &order, |p| {
                    let request = request.clone();
                    async move {
                        let series = p.history(&request).await?;
                        // A series must be exactly what was asked for, from one
                        // source. Anything else is rejected, not patched.
                        if series.data.id != request.id || series.data.interval != request.interval
                        {
                            return Err(DataError::Parse(
                                "provider returned a different series".into(),
                            ));
                        }
                        Ok(series)
                    }
                })
                .await
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use chrono::Utc;
    use nivesh_core::{Candle, FeedKind, Interval, Provenance};

    use super::*;
    use crate::HistoryRange;
    use crate::clock::ManualClock;
    use crate::health::HealthState;
    use crate::mock::MockProvider;

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
            Capabilities::QUOTE | Capabilities::HISTORY | Capabilities::INDEX,
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
