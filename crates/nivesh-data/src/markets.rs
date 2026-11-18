//! Derived market views built on the provider chain: breadth, the share of
//! stocks above their moving averages, and sector performance.
//!
//! Each result is a `Sourced<T>`. A view assembled from many requests keeps to
//! one provider (rows from any other are dropped, not mixed in) so its label
//! says exactly where the numbers came from.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures::FutureExt as _;
use futures::StreamExt as _;
use futures::future::join_all;
use nivesh_core::{ConstituentsInfo, IndexId, InstrumentId, Interval, Provenance, Sourced};
use serde::{Deserialize, Serialize};

use crate::analytics::{AboveMa, Breadth, Returns, above_moving_averages, breadth, period_returns};
use crate::cache::{Fetch, Ttl, cached};
use crate::{Capabilities, DataError, HistoryRange, HistoryRequest, MarketData};

/// Moving averages reported, in trading days.
pub const DMA_PERIODS: [u32; 3] = [20, 50, 200];

/// Daily history needed for the longest average (about 250 sessions a year).
const DMA_RANGE: HistoryRange = HistoryRange::Y1;

/// Parallel history requests for the breadth sweep. The shared HTTP client also
/// rate-limits per host; this only bounds how many wait at once.
const DMA_CONCURRENCY: usize = 4;

/// Breadth of one index: its stocks by direction of the day's move.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexBreadth {
    pub index: IndexId,
    pub breadth: Breadth,
}

/// Which universe the moving-average sweep covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DmaUniverse {
    Nifty50,
    /// NIFTY 50 plus NIFTY NEXT 50.
    Nifty100,
}

impl DmaUniverse {
    pub const fn code(self) -> &'static str {
        match self {
            DmaUniverse::Nifty50 => "NIFTY50",
            DmaUniverse::Nifty100 => "NIFTY100",
        }
    }
}

/// Share of a universe above its 20, 50 and 200-day moving averages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DmaBreadth {
    pub universe: DmaUniverse,
    /// Stocks in the universe's list.
    pub members: u32,
    pub periods: Vec<AboveMa>,
    /// The membership list used, with its date.
    pub members_info: ConstituentsInfo,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorRow {
    pub index: IndexId,
    pub last: f64,
    pub returns: Returns,
}

/// Returns of every catalogued index over 1D, 1W, 1M and year to date.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorPerformance {
    pub rows: Vec<SectorRow>,
}

/// Progress of the moving-average sweep: `(done, total)`.
pub type Progress = Arc<dyn Fn(usize, usize) + Send + Sync>;

impl MarketData {
    /// Advances, declines and unchanged for an index, from its constituent
    /// quotes (cached like the index view).
    pub async fn index_breadth(&self, index: IndexId) -> Result<Sourced<IndexBreadth>, DataError> {
        let snap = self.index_snapshot(index).await?;
        if snap.data.constituents.is_empty() {
            return Err(DataError::NotFound);
        }
        let counts = breadth(&snap.data.constituents);
        Ok(Sourced {
            data: IndexBreadth {
                index,
                breadth: counts,
            },
            provenance: snap.provenance,
        })
    }

    /// Returns of every index over 1D, 1W, 1M and YTD, from daily history.
    pub async fn sector_performance(&self) -> Result<Sourced<SectorPerformance>, DataError> {
        match self.cache_layer() {
            Some(layer) => {
                let md = self.clone();
                let fetch: Fetch<Sourced<SectorPerformance>> = Arc::new(move || {
                    let md = md.clone();
                    async move { md.fetch_sector_performance().await }.boxed()
                });
                let ttl = Ttl::new(Duration::from_secs(300), Duration::from_secs(900));
                cached(layer, "sp".to_owned(), "sectors", ttl, fetch).await
            }
            None => self.fetch_sector_performance().await,
        }
    }

    async fn fetch_sector_performance(&self) -> Result<Sourced<SectorPerformance>, DataError> {
        let order = self.history_order();
        let results = join_all(IndexId::all().map(|index| {
            let order = order.clone();
            async move {
                let result = self
                    .route(Capabilities::HISTORY, &order, |p| async move {
                        p.index_history(index, HistoryRange::Y1).await
                    })
                    .await;
                (index, result)
            }
        }))
        .await;

        let mut rows = Vec::new();
        let mut provenance: Option<Provenance> = None;
        let mut first_error: Option<DataError> = None;
        for (index, result) in results {
            match result {
                Ok(series) => {
                    // One provider per table: a row from another would mix sources.
                    match &mut provenance {
                        None => provenance = Some(series.provenance.clone()),
                        Some(p) if p.provider != series.provenance.provider => continue,
                        Some(p) => {
                            p.as_of = p.as_of.max(series.provenance.as_of);
                            p.stale |= series.provenance.stale;
                        }
                    }
                    let Some(last) = series.data.last().map(|c| c.c) else {
                        continue;
                    };
                    rows.push(SectorRow {
                        index,
                        last,
                        returns: period_returns(&series.data),
                    });
                }
                // The first failure is the cause; later ones may only say the
                // circuit is open.
                Err(e) => first_error = first_error.or(Some(e)),
            }
        }
        match provenance {
            Some(provenance) if !rows.is_empty() => Ok(Sourced {
                data: SectorPerformance { rows },
                provenance,
            }),
            _ => Err(first_error.unwrap_or(DataError::NotFound)),
        }
    }

    /// The share of NIFTY 50 or NIFTY 100 above their 20/50/200-day averages.
    ///
    /// Needs a year of daily candles per stock, so a cold run is 50 or 100
    /// history requests at the shared rate limit (a minute or so). Candles are
    /// cached (and kept in the store), so later runs are quick. `progress` is
    /// called as stocks complete, and not at all when the answer is cached.
    pub async fn dma_breadth(
        &self,
        universe: DmaUniverse,
        progress: Progress,
    ) -> Result<Sourced<DmaBreadth>, DataError> {
        match self.cache_layer() {
            Some(layer) => {
                let md = self.clone();
                let fetch: Fetch<Sourced<DmaBreadth>> = Arc::new(move || {
                    let (md, progress) = (md.clone(), Arc::clone(&progress));
                    async move { md.fetch_dma_breadth(universe, progress).await }.boxed()
                });
                let ttl = Ttl::new(Duration::from_secs(900), Duration::from_secs(6 * 3600));
                cached(
                    layer,
                    format!("dma:{}", universe.code()),
                    "breadth",
                    ttl,
                    fetch,
                )
                .await
            }
            None => self.fetch_dma_breadth(universe, progress).await,
        }
    }

    fn universe_members(
        &self,
        universe: DmaUniverse,
    ) -> Result<(Vec<InstrumentId>, ConstituentsInfo), DataError> {
        let lists = self.constituent_lists().ok_or(DataError::Unsupported)?;
        let (mut ids, info) = lists.members(IndexId::Nifty50).ok_or(DataError::NotFound)?;
        if universe == DmaUniverse::Nifty100 {
            let set = lists.snapshot();
            let next = set.lists.get("NIFTYNEXT50").ok_or(DataError::NotFound)?;
            for symbol in next {
                if let Ok(id) = InstrumentId::new(nivesh_core::Exchange::Nse, symbol)
                    && !ids.contains(&id)
                {
                    ids.push(id);
                }
            }
        }
        Ok((ids, info))
    }

    async fn fetch_dma_breadth(
        &self,
        universe: DmaUniverse,
        progress: Progress,
    ) -> Result<Sourced<DmaBreadth>, DataError> {
        let (ids, members_info) = self.universe_members(universe)?;
        let total = ids.len();
        let mut done = 0usize;
        progress(done, total);

        let mut stream = futures::stream::iter(ids.iter().cloned().map(|id| async move {
            self.history(&HistoryRequest {
                id,
                interval: Interval::D1,
                range: DMA_RANGE,
            })
            .await
        }))
        .buffer_unordered(DMA_CONCURRENCY);

        let mut closes: Vec<Vec<f64>> = Vec::new();
        let mut provenance: Option<Provenance> = None;
        let mut first_error: Option<DataError> = None;
        while let Some(result) = stream.next().await {
            done += 1;
            progress(done, total);
            match result {
                Ok(series) => {
                    match &mut provenance {
                        None => provenance = Some(series.provenance.clone()),
                        Some(p) if p.provider != series.provenance.provider => continue,
                        Some(p) => {
                            p.as_of = p.as_of.max(series.provenance.as_of);
                            p.stale |= series.provenance.stale;
                        }
                    }
                    closes.push(series.data.candles.iter().map(|c| c.c).collect());
                }
                // The first failure is the cause; later ones may only say the
                // circuit is open.
                Err(e) => first_error = first_error.or(Some(e)),
            }
        }
        let Some(provenance) = provenance else {
            return Err(first_error.unwrap_or(DataError::NotFound));
        };
        // A sweep that mostly failed would report a misleading percentage.
        if closes.len() * 2 < total {
            return Err(DataError::Upstream(format!(
                "only {} of {total} stocks' history could be fetched",
                closes.len()
            )));
        }
        Ok(Sourced {
            data: DmaBreadth {
                universe,
                members: u32::try_from(total).unwrap_or(u32::MAX),
                periods: above_moving_averages(&closes, &DMA_PERIODS),
                members_info,
            },
            provenance: Provenance {
                fetched_at: Utc::now(),
                ..provenance
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::Ordering;

    use chrono::Utc;
    use nivesh_core::{Candle, FeedKind, ProviderId};

    use super::*;
    use crate::constituents::ConstituentsList;
    use crate::mock::MockProvider;

    fn candles(closes: impl IntoIterator<Item = f64>) -> Vec<Candle> {
        // Consecutive IST sessions ending 2026-11-13, oldest first.
        let closes: Vec<f64> = closes.into_iter().collect();
        let n = closes.len() as i64;
        closes
            .into_iter()
            .enumerate()
            .map(|(i, c)| Candle {
                t: (1_794_767_400 - 3 * 86_400 + (i as i64 - n + 1) * 86_400 + 9 * 3600) * 1000,
                o: c,
                h: c,
                l: c,
                c,
                v: 1.0,
            })
            .collect()
    }

    fn market(p: &Arc<MockProvider>) -> MarketData {
        MarketData::builder()
            .provider(p.clone())
            .constituents(Arc::new(ConstituentsList::bundled().unwrap()))
            .build()
    }

    fn mock() -> Arc<MockProvider> {
        Arc::new(MockProvider::new(
            ProviderId::Yahoo,
            Capabilities::QUOTE | Capabilities::HISTORY | Capabilities::INDEX,
        ))
    }

    fn sourced(c: Vec<Candle>) -> Sourced<Vec<Candle>> {
        let now = Utc::now();
        Sourced::new(
            c,
            Provenance::new(ProviderId::Yahoo, nivesh_core::FeedKind::Delayed, now, now),
        )
    }

    #[tokio::test]
    async fn sector_performance_covers_every_index_from_one_provider() {
        let p = mock();
        p.repeat_index_history(Ok(sourced(candles((0..30).map(|i| 100.0 + f64::from(i))))));
        let sp = market(&p).sector_performance().await.unwrap();
        assert_eq!(sp.data.rows.len(), IndexId::all().count());
        assert_eq!(sp.provenance.provider, ProviderId::Yahoo);
        assert_eq!(sp.provenance.feed, FeedKind::Delayed);
        let row = &sp.data.rows[0];
        assert_eq!(row.last, 129.0);
        // 129 against 128 the day before.
        assert!((row.returns.d1.unwrap() - (129.0 / 128.0 - 1.0) * 100.0).abs() < 1e-9);
        assert!(row.returns.w1.unwrap() > 0.0);
        // Only 30 days of history: no year-to-date base.
        assert!(row.returns.ytd.is_none());
    }

    #[tokio::test]
    async fn sector_performance_fails_with_the_providers_error_when_nothing_answers() {
        let p = mock();
        p.repeat_index_history(Err(DataError::Blocked));
        assert_eq!(
            market(&p).sector_performance().await.unwrap_err(),
            DataError::Blocked
        );
    }

    #[tokio::test]
    async fn index_breadth_counts_the_constituents() {
        use nivesh_core::{Exchange, IndexQuote, IndexSnapshot, Quote};
        let p = mock();
        let q = |sym: &str, ltp: f64| Quote {
            id: nivesh_core::InstrumentId::new(Exchange::Nse, sym).unwrap(),
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
        };
        let now = Utc::now();
        p.repeat_index(Ok(Sourced::new(
            IndexSnapshot {
                quote: IndexQuote {
                    index: IndexId::Nifty50,
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
                constituents: vec![q("A", 101.0), q("B", 99.0), q("C", 100.0), q("D", 102.0)],
                constituents_info: None,
            },
            Provenance::new(ProviderId::Yahoo, FeedKind::Delayed, now, now),
        )));
        let b = market(&p).index_breadth(IndexId::Nifty50).await.unwrap();
        assert_eq!(
            (
                b.data.breadth.advances,
                b.data.breadth.declines,
                b.data.breadth.unchanged
            ),
            (2, 1, 1)
        );
        assert_eq!(b.provenance.provider, ProviderId::Yahoo);
    }

    #[tokio::test]
    async fn dma_breadth_sweeps_the_universe_and_reports_progress() {
        let p = mock();
        // A steadily rising stock closes above every average.
        p.echo_history(candles((0..250).map(|i| 100.0 + f64::from(i))));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        let progress: Progress =
            Arc::new(move |done, total| log.lock().unwrap().push((done, total)));
        let r = market(&p)
            .dma_breadth(DmaUniverse::Nifty50, progress)
            .await
            .unwrap();
        assert_eq!(r.data.members, 50);
        assert_eq!(MockProvider::calls(&p.history_calls), 50);
        for period in &r.data.periods {
            assert_eq!((period.measured, period.above), (50, 50), "{period:?}");
            assert_eq!(period.percent(), Some(100.0));
        }
        assert_eq!(
            r.data.periods.iter().map(|p| p.period).collect::<Vec<_>>(),
            DMA_PERIODS
        );
        assert_eq!(r.provenance.provider, ProviderId::Yahoo);
        assert_eq!(r.data.members_info.source, "NSE Indices (niftyindices.com)");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.first(), Some(&(0, 50)));
        assert_eq!(seen.last(), Some(&(50, 50)));
    }

    #[tokio::test]
    async fn dma_breadth_for_nifty_100_covers_both_lists_once() {
        let p = mock();
        // 150 closes: measurable for 20 and 50 days, not 200.
        p.echo_history(candles((0..150).map(|i| 500.0 - f64::from(i))));
        let r = market(&p)
            .dma_breadth(DmaUniverse::Nifty100, Arc::new(|_, _| {}))
            .await
            .unwrap();
        assert_eq!(r.data.members, 100);
        assert_eq!(p.history_calls.load(Ordering::SeqCst), 100);
        let by = |n: u32| r.data.periods.iter().find(|p| p.period == n).unwrap();
        assert_eq!((by(20).measured, by(20).above), (100, 0), "falling stocks");
        assert_eq!(by(200).measured, 0, "too little history to measure");
        assert_eq!(by(200).percent(), None);
    }

    #[tokio::test]
    async fn a_mostly_failed_sweep_is_an_error_not_a_misleading_percentage() {
        let p = mock();
        p.repeat_history(Err(DataError::Timeout));
        let err = market(&p)
            .dma_breadth(DmaUniverse::Nifty50, Arc::new(|_, _| {}))
            .await
            .unwrap_err();
        assert_eq!(err, DataError::Timeout);
    }
}
