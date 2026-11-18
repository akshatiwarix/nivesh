//! Builds the market-data services and the event bridges. This is wiring:
//! it constructs `nivesh-data` pieces and connects them to Tauri. The logic
//! lives in the crate.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context as _;
use nivesh_core::{MarketCalendar, ProviderId, Session};
use nivesh_data::constituents::{self, ConstituentsList};
use nivesh_data::http::{HostLimits, HttpClient, HttpConfig};
use nivesh_data::nse::{NseConfig, NseProvider};
use nivesh_data::yahoo::YahooProvider;
use nivesh_data::{
    CacheConfig, DataError, DataProvider, HubConfig, InstrumentMaster, MarketData, QuoteHub,
};
use nivesh_store::Store;
use tauri::{AppHandle, Emitter as _};
use tokio::sync::broadcast::error::RecvError;

use crate::commands::market::MarketStatusDto;
use crate::ipc::dto::ProviderStatusDto;

/// Everything the market-data commands need.
pub struct Services {
    pub market: MarketData,
    pub hub: QuoteHub,
    pub instruments: Arc<InstrumentMaster>,
    /// The index membership lists (bundled, or a newer download).
    pub constituents: Arc<ConstituentsList>,
    /// Providers that can supply instrument lists, for the daily refresh.
    list_sources: Vec<Arc<dyn DataProvider>>,
    /// How to download the membership lists; `None` in tests.
    list_download: Option<ListDownload>,
}

struct ListDownload {
    http: HttpClient,
    bases: Vec<String>,
}

/// Cached market data older than this is pruned at startup.
const CACHE_KEEP: Duration = Duration::from_secs(14 * 24 * 3600);

/// How often the instrument list is checked for a due refresh.
const LIST_CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);

/// Debug-build hooks for local testing. Release builds ignore them.
///
/// * `NIVESH_NSE_URL` / `NIVESH_YAHOO_URL`: use a local server (fixtures)
///   instead of the real providers.
/// * `NIVESH_FORCE_FAIL=nse,yahoo`: make those providers fail as if blocked,
///   to watch the fallback and the status bar react.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Overrides {
    pub nse_url: Option<String>,
    pub yahoo_url: Option<String>,
    pub force_fail: Vec<ProviderId>,
}

impl Overrides {
    pub fn from_env() -> Self {
        if !cfg!(debug_assertions) {
            return Self::default();
        }
        Self::from_vars(|k| std::env::var(k).ok())
    }

    fn from_vars(get: impl Fn(&str) -> Option<String>) -> Self {
        let force_fail = get("NIVESH_FORCE_FAIL")
            .unwrap_or_default()
            .split(',')
            .filter_map(|name| match name.trim().to_ascii_lowercase().as_str() {
                "nse" => Some(ProviderId::Nse),
                "yahoo" => Some(ProviderId::Yahoo),
                _ => None,
            })
            .collect();
        Self {
            nse_url: get("NIVESH_NSE_URL").filter(|v| !v.is_empty()),
            yahoo_url: get("NIVESH_YAHOO_URL").filter(|v| !v.is_empty()),
            force_fail,
        }
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or_default()
}

/// Builds the real providers and the services around them. Must run inside the
/// tokio runtime (the hub spawns its poller).
pub fn build(store: Arc<Store>, overrides: &Overrides) -> anyhow::Result<Services> {
    let limits = Arc::new(HostLimits::defaults());
    // NSE needs a cookie jar of its own; Yahoo doesn't want one.
    let nse_http = HttpClient::new(
        HttpConfig {
            cookies: true,
            ..HttpConfig::default()
        },
        Arc::clone(&limits),
    )
    .context("building NSE http client")?;
    let yahoo_http =
        HttpClient::new(HttpConfig::default(), limits).context("building Yahoo http client")?;
    // Index membership files are static downloads: no cookies, own client.
    let list_download = ListDownload {
        http: yahoo_http.clone(),
        bases: match &overrides.nse_url {
            Some(url) => vec![format!("{}/content/indices", url.trim_end_matches('/'))],
            None => constituents::DEFAULT_BASES
                .iter()
                .map(|b| (*b).to_owned())
                .collect(),
        },
    };

    let nse = match &overrides.nse_url {
        Some(url) => {
            NseProvider::with_base_url(nse_http, url, NseConfig::default()).with_archive_url(url)
        }
        None => NseProvider::new(nse_http),
    };
    let yahoo = match &overrides.yahoo_url {
        Some(url) => YahooProvider::with_base_url(yahoo_http, url),
        None => YahooProvider::new(yahoo_http),
    };
    let nse: Arc<dyn DataProvider> = Arc::new(nse);
    let yahoo: Arc<dyn DataProvider> = Arc::new(yahoo);
    assemble(
        store,
        vec![nse, yahoo],
        &overrides.force_fail,
        Some(list_download),
    )
}

/// Wires services around any providers (tests pass mocks).
pub fn build_with_providers(
    store: Arc<Store>,
    providers: Vec<Arc<dyn DataProvider>>,
    force_fail: &[ProviderId],
) -> anyhow::Result<Services> {
    assemble(store, providers, force_fail, None)
}

fn assemble(
    store: Arc<Store>,
    providers: Vec<Arc<dyn DataProvider>>,
    force_fail: &[ProviderId],
    list_download: Option<ListDownload>,
) -> anyhow::Result<Services> {
    let cutoff_ms = (now_secs() - i64::try_from(CACHE_KEEP.as_secs()).unwrap_or(i64::MAX)) * 1000;
    if let Err(err) = store.cache_prune(cutoff_ms) {
        tracing::warn!(%err, "pruning the data cache failed");
    }

    let constituents = Arc::new(
        ConstituentsList::open(Arc::clone(&store)).context("opening the constituent lists")?,
    );
    let mut builder = MarketData::builder()
        .constituents(Arc::clone(&constituents))
        .cache(CacheConfig {
            persistent: Some(Arc::clone(&store) as Arc<dyn nivesh_data::PersistentCache>),
            ..CacheConfig::default()
        });
    for provider in &providers {
        builder = builder.provider(Arc::clone(provider));
    }
    let market = builder.build();
    for provider in force_fail {
        tracing::warn!(%provider, "debug hook: forcing provider to fail");
        market.force_failure(*provider, Some(DataError::Blocked));
    }

    let instruments = Arc::new(
        InstrumentMaster::open_ranked(store, constituents.popularity())
            .context("opening the instrument master")?,
    );
    let calendar = Arc::new(MarketCalendar::bundled());
    let hub = QuoteHub::spawn(market.clone(), HubConfig::default(), move || {
        calendar.status_at(now_secs()).session != Session::Closed
    });
    Ok(Services {
        market,
        hub,
        instruments,
        constituents,
        list_sources: providers,
        list_download,
    })
}

impl Services {
    /// Background work: the daily instrument-list refresh and the weekly
    /// index-membership refresh.
    pub fn spawn_background_tasks(self: &Arc<Self>) {
        let services = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            loop {
                let now_ms = now_secs() * 1000;
                services
                    .instruments
                    .refresh_if_due(&services.list_sources, now_ms)
                    .await;
                if let Some(download) = &services.list_download {
                    let changed = services
                        .constituents
                        .refresh_if_due(&download.http, &download.bases, now_ms)
                        .await;
                    if changed
                        && let Err(err) = services
                            .instruments
                            .set_popularity(services.constituents.popularity())
                    {
                        tracing::warn!(%err, "re-ranking search after a constituent refresh failed");
                    }
                }
                tokio::time::sleep(LIST_CHECK_EVERY).await;
            }
        });
    }

    /// Forwards health changes and market-session changes to the webview as
    /// `provider:status` and `market:session` events.
    pub fn spawn_event_bridges(self: &Arc<Self>, app: &AppHandle) {
        let mut status_rx = self.market.subscribe_status();
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                match status_rx.recv().await {
                    Ok(status) => {
                        if let Err(err) =
                            handle.emit("provider:status", ProviderStatusDto::from(&status))
                        {
                            tracing::debug!(%err, "provider:status emit failed");
                        }
                    }
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                }
            }
        });

        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            let calendar = MarketCalendar::bundled();
            let mut last = None;
            loop {
                let now = now_secs();
                let status = calendar.status_at(now);
                let key = (status.session, status.holiday.clone());
                if last.as_ref() != Some(&key) {
                    last = Some(key);
                    if let Err(err) =
                        handle.emit("market:session", MarketStatusDto::from(status.clone()))
                    {
                        tracing::debug!(%err, "market:session emit failed");
                    }
                }
                // Wake at the next boundary, but at least every 30 s so a clock
                // change or a sleeping laptop is noticed soon.
                let until_change = u64::try_from(status.next_change_utc - now)
                    .unwrap_or(1)
                    .clamp(1, 30);
                tokio::time::sleep(Duration::from_secs(until_change)).await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::Utc;
    use nivesh_core::{Exchange, FeedKind, InstrumentId, Provenance, Quote, Sourced};
    use nivesh_data::mock::MockProvider;
    use nivesh_data::{Capabilities, HubUpdate, Topic};
    use tokio::sync::mpsc;

    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn overrides_parse_urls_and_forced_failures() {
        let o = Overrides::from_vars(vars(&[
            ("NIVESH_NSE_URL", "http://127.0.0.1:9000"),
            ("NIVESH_FORCE_FAIL", "NSE, yahoo ,bogus"),
        ]));
        assert_eq!(o.nse_url.as_deref(), Some("http://127.0.0.1:9000"));
        assert_eq!(o.yahoo_url, None);
        assert_eq!(o.force_fail, [ProviderId::Nse, ProviderId::Yahoo]);
        assert_eq!(Overrides::from_vars(vars(&[])), Overrides::default());
        assert_eq!(
            Overrides::from_vars(vars(&[("NIVESH_NSE_URL", "")])).nse_url,
            None
        );
    }

    #[test]
    fn debug_hooks_are_inert_in_release_builds() {
        // `from_env` returns defaults unless debug assertions are on.
        if !cfg!(debug_assertions) {
            assert_eq!(Overrides::from_env(), Overrides::default());
        }
    }

    fn quote(provider: ProviderId, sym: &str) -> Sourced<Quote> {
        let now = Utc::now();
        Sourced::new(
            Quote {
                id: InstrumentId::new(Exchange::Nse, sym).unwrap(),
                name: None,
                ltp: 10.0,
                prev_close: 9.0,
                open: None,
                high: None,
                low: None,
                volume: None,
                week52_high: None,
                week52_low: None,
                upper_band: None,
                lower_band: None,
            },
            Provenance::new(provider, FeedKind::Delayed, now, now),
        )
    }

    #[tokio::test]
    async fn services_search_locally_and_stream_quotes_through_the_hub() {
        let nse = Arc::new(MockProvider::new(ProviderId::Nse, Capabilities::QUOTE));
        nse.repeat_quotes(Ok(vec![quote(ProviderId::Nse, "RELIANCE")]));
        let store = Arc::new(Store::open_in_memory().unwrap());
        let services = build_with_providers(store, vec![nse], &[]).unwrap();

        // Search needs no network: it is served from the seeded master.
        let hits = services.instruments.search("relia", 5);
        assert!(hits.iter().any(|i| i.id.symbol() == "RELIANCE"));

        let (tx, mut rx) = mpsc::unbounded_channel();
        let id = InstrumentId::new(Exchange::Nse, "RELIANCE").unwrap();
        let sub = services.hub.subscribe([Topic::Quote(id)], tx);
        let update = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(update, HubUpdate::Quotes(q) if q[0].provenance.provider == ProviderId::Nse)
        );
        services.hub.unsubscribe(sub);
        services.hub.shutdown();
    }

    #[tokio::test]
    async fn blocked_nse_still_yields_nifty_constituents_from_the_bundled_list() {
        use nivesh_core::{IndexId, IndexQuote, IndexSnapshot};

        let nse = Arc::new(MockProvider::new(ProviderId::Nse, Capabilities::INDEX));
        let yahoo = Arc::new(MockProvider::new(
            ProviderId::Yahoo,
            Capabilities::QUOTE | Capabilities::INDEX,
        ));
        let now = Utc::now();
        let prov = Provenance::new(ProviderId::Yahoo, FeedKind::Delayed, now, now);
        yahoo.repeat_index(Ok(Sourced::new(
            IndexSnapshot {
                quote: IndexQuote {
                    index: IndexId::Nifty50,
                    last: 25000.0,
                    prev_close: 24900.0,
                    open: None,
                    high: None,
                    low: None,
                    week52_high: None,
                    week52_low: None,
                    advances: None,
                    declines: None,
                },
                constituents: vec![],
                constituents_info: None,
            },
            prov,
        )));
        yahoo.repeat_quotes(Ok(vec![quote(ProviderId::Yahoo, "RELIANCE")]));
        let store = Arc::new(Store::open_in_memory().unwrap());
        let services = build_with_providers(store, vec![nse, yahoo], &[ProviderId::Nse]).unwrap();
        let snap = services
            .market
            .index_snapshot(IndexId::Nifty50)
            .await
            .unwrap();
        assert_eq!(snap.provenance.provider, ProviderId::Yahoo);
        assert_eq!(snap.data.constituents.len(), 1);
        let info = snap.data.constituents_info.unwrap();
        assert_eq!(info.as_of, services.constituents.snapshot().as_of);
        services.hub.shutdown();
    }

    #[tokio::test]
    async fn a_forced_nse_failure_falls_back_to_yahoo() {
        let nse = Arc::new(MockProvider::new(ProviderId::Nse, Capabilities::QUOTE));
        nse.repeat_quotes(Ok(vec![quote(ProviderId::Nse, "TCS")]));
        let yahoo = Arc::new(MockProvider::new(ProviderId::Yahoo, Capabilities::QUOTE));
        yahoo.repeat_quotes(Ok(vec![quote(ProviderId::Yahoo, "TCS")]));
        let store = Arc::new(Store::open_in_memory().unwrap());
        let services = build_with_providers(store, vec![nse, yahoo], &[ProviderId::Nse]).unwrap();
        let id = InstrumentId::new(Exchange::Nse, "TCS").unwrap();
        let q = services.market.quote(&id).await.unwrap();
        assert_eq!(q.provenance.provider, ProviderId::Yahoo);
        services.hub.shutdown();
    }
}
