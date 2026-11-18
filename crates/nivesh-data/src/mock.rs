//! A scripted provider for tests, here and in other crates.
//!
//! Queue results per method with the `push_*` methods; when a method's queue is
//! empty it returns the `repeat_*` result if one is set, otherwise
//! `Unsupported`. Calls are counted so tests can assert how often a provider
//! was hit.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use nivesh_core::{
    CandleSeries, FeedKind, IndexId, IndexQuote, IndexSnapshot, InstrumentId, ProviderId, Quote,
    Sourced,
};

use crate::{Capabilities, DataError, DataProvider, HistoryRequest};

struct Script<T> {
    queue: VecDeque<Result<T, DataError>>,
    repeat: Option<Result<T, DataError>>,
}

impl<T: Clone> Script<T> {
    fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            repeat: None,
        }
    }

    fn next(&mut self) -> Result<T, DataError> {
        self.queue
            .pop_front()
            .or_else(|| self.repeat.clone())
            .unwrap_or(Err(DataError::Unsupported))
    }
}

type Quotes = Vec<Sourced<Quote>>;

pub struct MockProvider {
    id: ProviderId,
    capabilities: Capabilities,
    feed: FeedKind,
    delay: Duration,
    quotes: Mutex<Script<Quotes>>,
    history: Mutex<Script<Sourced<CandleSeries>>>,
    index: Mutex<Script<Sourced<IndexSnapshot>>>,
    all_indices: Mutex<Script<Sourced<Vec<IndexQuote>>>>,
    pub quote_calls: AtomicUsize,
    pub history_calls: AtomicUsize,
    pub index_calls: AtomicUsize,
    pub all_indices_calls: AtomicUsize,
    quote_requests: Mutex<Vec<Vec<InstrumentId>>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl MockProvider {
    pub fn new(id: ProviderId, capabilities: Capabilities) -> Self {
        Self {
            id,
            capabilities,
            feed: FeedKind::Delayed,
            delay: Duration::ZERO,
            quotes: Mutex::new(Script::new()),
            history: Mutex::new(Script::new()),
            index: Mutex::new(Script::new()),
            all_indices: Mutex::new(Script::new()),
            quote_calls: AtomicUsize::new(0),
            history_calls: AtomicUsize::new(0),
            index_calls: AtomicUsize::new(0),
            all_indices_calls: AtomicUsize::new(0),
            quote_requests: Mutex::new(Vec::new()),
        }
    }

    /// Every call takes this long (tokio time, so tests can pause the clock).
    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    pub fn with_feed(mut self, feed: FeedKind) -> Self {
        self.feed = feed;
        self
    }

    pub fn push_quotes(&self, result: Result<Quotes, DataError>) {
        lock(&self.quotes).queue.push_back(result);
    }

    pub fn repeat_quotes(&self, result: Result<Quotes, DataError>) {
        lock(&self.quotes).repeat = Some(result);
    }

    pub fn push_history(&self, result: Result<Sourced<CandleSeries>, DataError>) {
        lock(&self.history).queue.push_back(result);
    }

    pub fn repeat_history(&self, result: Result<Sourced<CandleSeries>, DataError>) {
        lock(&self.history).repeat = Some(result);
    }

    pub fn push_index(&self, result: Result<Sourced<IndexSnapshot>, DataError>) {
        lock(&self.index).queue.push_back(result);
    }

    pub fn repeat_index(&self, result: Result<Sourced<IndexSnapshot>, DataError>) {
        lock(&self.index).repeat = Some(result);
    }

    pub fn repeat_all_indices(&self, result: Result<Sourced<Vec<IndexQuote>>, DataError>) {
        lock(&self.all_indices).repeat = Some(result);
    }

    /// Ids asked for in each `quotes` call, in order.
    pub fn quote_requests(&self) -> Vec<Vec<InstrumentId>> {
        lock(&self.quote_requests).clone()
    }

    pub fn calls(counter: &AtomicUsize) -> usize {
        counter.load(Ordering::SeqCst)
    }

    async fn pause(&self) {
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
    }
}

#[async_trait]
impl DataProvider for MockProvider {
    fn id(&self) -> ProviderId {
        self.id
    }

    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    fn feed_kind(&self) -> FeedKind {
        self.feed
    }

    async fn quotes(&self, ids: &[InstrumentId]) -> Result<Quotes, DataError> {
        self.quote_calls.fetch_add(1, Ordering::SeqCst);
        lock(&self.quote_requests).push(ids.to_vec());
        self.pause().await;
        lock(&self.quotes).next()
    }

    async fn history(&self, _req: &HistoryRequest) -> Result<Sourced<CandleSeries>, DataError> {
        self.history_calls.fetch_add(1, Ordering::SeqCst);
        self.pause().await;
        lock(&self.history).next()
    }

    async fn index_snapshot(&self, _index: IndexId) -> Result<Sourced<IndexSnapshot>, DataError> {
        self.index_calls.fetch_add(1, Ordering::SeqCst);
        self.pause().await;
        lock(&self.index).next()
    }

    async fn all_indices(&self) -> Result<Sourced<Vec<IndexQuote>>, DataError> {
        self.all_indices_calls.fetch_add(1, Ordering::SeqCst);
        self.pause().await;
        lock(&self.all_indices).next()
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use nivesh_core::{Exchange, Provenance};

    use super::*;

    fn quote(sym: &str) -> Sourced<Quote> {
        let now = Utc::now();
        Sourced::new(
            Quote {
                id: InstrumentId::new(Exchange::Nse, sym).unwrap(),
                name: None,
                ltp: 1.0,
                prev_close: 1.0,
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

    #[tokio::test]
    async fn queue_then_repeat_then_unsupported() {
        let p = MockProvider::new(ProviderId::Nse, Capabilities::QUOTE);
        let id = InstrumentId::new(Exchange::Nse, "TCS").unwrap();
        p.push_quotes(Err(DataError::Timeout));
        p.repeat_quotes(Ok(vec![quote("TCS")]));
        assert_eq!(
            p.quotes(std::slice::from_ref(&id)).await.unwrap_err(),
            DataError::Timeout
        );
        assert_eq!(p.quotes(std::slice::from_ref(&id)).await.unwrap().len(), 1);
        assert_eq!(p.quotes(std::slice::from_ref(&id)).await.unwrap().len(), 1);
        assert_eq!(MockProvider::calls(&p.quote_calls), 3);
        assert_eq!(p.quote_requests().len(), 3);

        let fresh = MockProvider::new(ProviderId::Yahoo, Capabilities::QUOTE);
        assert_eq!(
            fresh.quotes(&[id]).await.unwrap_err(),
            DataError::Unsupported
        );
    }
}
