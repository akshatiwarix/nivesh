//! The quote hub: one poller behind every open view.
//!
//! Views subscribe to *topics* (a stock's quote, an index with its
//! constituents, the dashboard's index list). The hub counts subscribers per
//! topic and polls only topics somebody is showing, batching all stock quotes
//! into one request. It polls quickly while the market is open and slowly when
//! it is closed, and stops entirely while the window is hidden. A subscription
//! whose receiver has gone away is dropped automatically.
//!
//! The hub only schedules; fetching, fallback and caching are `MarketData`'s.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::FutureExt as _;
use nivesh_core::{IndexId, IndexQuote, IndexSnapshot, InstrumentId, ProviderId, Quote, Sourced};
use tokio::sync::{Notify, mpsc};

use crate::{DataError, MarketData};

/// What a view can subscribe to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Topic {
    Quote(InstrumentId),
    Index(IndexId),
    /// The index levels on the MKT dashboard.
    AllIndices,
    /// A quote from one named provider, never another. A chart's forming bar
    /// uses this so its last price comes from the source of its history.
    Pinned(ProviderId, InstrumentId),
}

/// A message to a subscriber.
#[derive(Debug, Clone, PartialEq)]
pub enum HubUpdate {
    /// Fresh quotes for the stocks this subscription asked for.
    Quotes(Vec<Sourced<Quote>>),
    Index(Sourced<IndexSnapshot>),
    AllIndices(Vec<Sourced<IndexQuote>>),
    /// The quote for a [`Topic::Pinned`] topic.
    Pinned(Sourced<Quote>),
    /// A topic couldn't be refreshed. The view shows this inline and keeps its
    /// last data; the hub keeps trying.
    Failed {
        topic: Topic,
        error: DataError,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SubscriptionId(pub u64);

#[derive(Debug, Clone, Copy)]
pub struct HubConfig {
    /// Poll period while the market is open. NSE guidance is 5-15 s.
    pub open_interval: Duration,
    /// Poll period while it is closed: prices barely move.
    pub closed_interval: Duration,
    /// New subscriptions arriving together trigger one poll, this long after
    /// the first.
    pub settle: Duration,
}

impl Default for HubConfig {
    fn default() -> Self {
        Self {
            open_interval: Duration::from_secs(5),
            closed_interval: Duration::from_secs(60),
            settle: Duration::from_millis(50),
        }
    }
}

struct Subscription {
    topics: HashSet<Topic>,
    sink: mpsc::UnboundedSender<HubUpdate>,
}

#[derive(Default)]
struct State {
    next_id: u64,
    subs: HashMap<SubscriptionId, Subscription>,
    refcount: HashMap<Topic, usize>,
    /// The most recent update per topic, so a new subscriber has something to
    /// show at once.
    latest: HashMap<Topic, HubUpdate>,
    hidden: bool,
    shutdown: bool,
}

struct Inner {
    market: MarketData,
    config: HubConfig,
    market_open: Arc<dyn Fn() -> bool + Send + Sync>,
    state: Mutex<State>,
    wake: Notify,
}

#[derive(Clone)]
pub struct QuoteHub {
    inner: Arc<Inner>,
}

fn lock(m: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl QuoteHub {
    /// Starts the poller on the current tokio runtime. `market_open` says
    /// whether to use the fast or the slow interval.
    pub fn spawn(
        market: MarketData,
        config: HubConfig,
        market_open: impl Fn() -> bool + Send + Sync + 'static,
    ) -> Self {
        let inner = Arc::new(Inner {
            market,
            config,
            market_open: Arc::new(market_open),
            state: Mutex::new(State::default()),
            wake: Notify::new(),
        });
        tokio::spawn(run(Arc::clone(&inner)));
        Self { inner }
    }

    /// Subscribes `sink` to `topics`. Whatever the hub already knows about
    /// them is sent immediately; the rest follows on the next poll.
    pub fn subscribe(
        &self,
        topics: impl IntoIterator<Item = Topic>,
        sink: mpsc::UnboundedSender<HubUpdate>,
    ) -> SubscriptionId {
        let topics: HashSet<Topic> = topics.into_iter().collect();
        let mut state = lock(&self.inner.state);
        state.next_id += 1;
        let id = SubscriptionId(state.next_id);
        for topic in &topics {
            *state.refcount.entry(topic.clone()).or_insert(0) += 1;
        }
        // Replay what is already known, grouping stock quotes into one message.
        let mut quotes = Vec::new();
        for topic in &topics {
            match state.latest.get(topic) {
                Some(HubUpdate::Quotes(q)) => quotes.extend(q.iter().cloned()),
                Some(update) => {
                    let _ = sink.send(update.clone());
                }
                None => {}
            }
        }
        if !quotes.is_empty() {
            let _ = sink.send(HubUpdate::Quotes(quotes));
        }
        state.subs.insert(id, Subscription { topics, sink });
        drop(state);
        self.inner.wake.notify_one();
        id
    }

    pub fn unsubscribe(&self, id: SubscriptionId) {
        remove_subscription(&mut lock(&self.inner.state), id);
    }

    /// Pauses polling while the window is hidden; resumes (with an immediate
    /// poll) when it is shown again.
    pub fn set_visible(&self, visible: bool) {
        lock(&self.inner.state).hidden = !visible;
        self.inner.wake.notify_one();
    }

    /// Number of distinct topics being polled.
    pub fn topic_count(&self) -> usize {
        lock(&self.inner.state).refcount.len()
    }

    pub fn subscription_count(&self) -> usize {
        lock(&self.inner.state).subs.len()
    }

    /// Stops the poller. Further subscriptions are accepted but never polled.
    pub fn shutdown(&self) {
        lock(&self.inner.state).shutdown = true;
        self.inner.wake.notify_one();
    }
}

fn remove_subscription(state: &mut State, id: SubscriptionId) {
    let Some(sub) = state.subs.remove(&id) else {
        return;
    };
    for topic in sub.topics {
        if let Some(n) = state.refcount.get_mut(&topic) {
            *n -= 1;
            if *n == 0 {
                state.refcount.remove(&topic);
                state.latest.remove(&topic);
            }
        }
    }
}

/// The poll loop.
async fn run(inner: Arc<Inner>) {
    loop {
        // Idle until there is something to show and someone to show it to.
        let topics = loop {
            let wake = inner.wake.notified();
            {
                let state = lock(&inner.state);
                if state.shutdown {
                    return;
                }
                if !state.hidden && !state.refcount.is_empty() {
                    break state.refcount.keys().cloned().collect::<Vec<_>>();
                }
            }
            wake.await;
        };

        // A wake-up that arrived before this poll is about to be satisfied by
        // it. Consume it so it doesn't trigger a redundant second poll.
        let _ = inner.wake.notified().now_or_never();
        poll(&inner, topics).await;

        let interval = if (inner.market_open)() {
            inner.config.open_interval
        } else {
            inner.config.closed_interval
        };
        // Sleep until the next poll, but wake early for a new subscription
        // (poll soon so it isn't left blank) or for hide/shutdown.
        let wake = inner.wake.notified();
        tokio::select! {
            () = tokio::time::sleep(interval) => {}
            () = wake => {
                let (hidden, shutdown) = {
                    let state = lock(&inner.state);
                    (state.hidden, state.shutdown)
                };
                if shutdown {
                    return;
                }
                if !hidden {
                    // Let a burst of subscriptions land, then poll once.
                    tokio::time::sleep(inner.config.settle).await;
                }
            }
        }
    }
}

/// Fetches every topic once and delivers the results.
async fn poll(inner: &Inner, topics: Vec<Topic>) {
    let ids: Vec<InstrumentId> = topics
        .iter()
        .filter_map(|t| match t {
            Topic::Quote(id) => Some(id.clone()),
            _ => None,
        })
        .collect();

    if !ids.is_empty() {
        match inner.market.quotes(&ids).await {
            Ok(quotes) => deliver_quotes(inner, &ids, quotes),
            Err(error) => {
                for id in ids {
                    deliver_failure(inner, Topic::Quote(id), error.clone());
                }
            }
        }
    }
    for topic in topics {
        match &topic {
            Topic::Quote(_) => {}
            Topic::Index(index) => match inner.market.index_snapshot(*index).await {
                Ok(snap) => deliver(inner, topic.clone(), HubUpdate::Index(snap)),
                Err(error) => deliver_failure(inner, topic.clone(), error),
            },
            Topic::AllIndices => match inner.market.all_indices().await {
                Ok(rows) => deliver(inner, topic.clone(), HubUpdate::AllIndices(rows)),
                Err(error) => deliver_failure(inner, topic.clone(), error),
            },
            Topic::Pinned(provider, id) => match inner.market.quote_from(*provider, id).await {
                Ok(q) => deliver(inner, topic.clone(), HubUpdate::Pinned(q)),
                Err(error) => deliver_failure(inner, topic.clone(), error),
            },
        }
    }
}

/// Sends each subscriber only the quotes it asked for.
fn deliver_quotes(inner: &Inner, asked: &[InstrumentId], quotes: Vec<Sourced<Quote>>) {
    let mut state = lock(&inner.state);
    for id in asked {
        let topic = Topic::Quote(id.clone());
        if let Some(q) = quotes.iter().find(|q| &q.data.id == id) {
            state
                .latest
                .insert(topic, HubUpdate::Quotes(vec![q.clone()]));
        }
    }
    let mut dead = Vec::new();
    for (sub_id, sub) in &state.subs {
        let mine: Vec<_> = quotes
            .iter()
            .filter(|q| sub.topics.contains(&Topic::Quote(q.data.id.clone())))
            .cloned()
            .collect();
        if !mine.is_empty() && sub.sink.send(HubUpdate::Quotes(mine)).is_err() {
            dead.push(*sub_id);
        }
    }
    for id in dead {
        remove_subscription(&mut state, id);
    }
}

fn deliver(inner: &Inner, topic: Topic, update: HubUpdate) {
    let mut state = lock(&inner.state);
    state.latest.insert(topic.clone(), update.clone());
    send_to_subscribers(&mut state, &topic, update);
}

fn deliver_failure(inner: &Inner, topic: Topic, error: DataError) {
    let mut state = lock(&inner.state);
    send_to_subscribers(
        &mut state,
        &topic.clone(),
        HubUpdate::Failed { topic, error },
    );
}

fn send_to_subscribers(state: &mut State, topic: &Topic, update: HubUpdate) {
    let dead: Vec<_> = state
        .subs
        .iter()
        .filter(|(_, sub)| sub.topics.contains(topic))
        .filter(|(_, sub)| sub.sink.send(update.clone()).is_err())
        .map(|(id, _)| *id)
        .collect();
    for id in dead {
        remove_subscription(state, id);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use chrono::Utc;
    use nivesh_core::{Exchange, FeedKind, Provenance, ProviderId};

    use super::*;
    use crate::Capabilities;
    use crate::mock::MockProvider;

    fn id(s: &str) -> InstrumentId {
        InstrumentId::new(Exchange::Nse, s).unwrap()
    }

    fn quote(sym: &str) -> Sourced<Quote> {
        let now = Utc::now();
        Sourced::new(
            Quote {
                id: id(sym),
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

    struct Rig {
        hub: QuoteHub,
        p: Arc<MockProvider>,
        open: Arc<AtomicBool>,
    }

    fn rig() -> Rig {
        let p = Arc::new(MockProvider::new(
            ProviderId::Nse,
            Capabilities::QUOTE | Capabilities::INDEX,
        ));
        p.repeat_quotes(Ok(vec![quote("TCS"), quote("INFY")]));
        let md = MarketData::builder().provider(p.clone()).build();
        let open = Arc::new(AtomicBool::new(true));
        let flag = Arc::clone(&open);
        let hub = QuoteHub::spawn(md, HubConfig::default(), move || {
            flag.load(Ordering::SeqCst)
        });
        Rig { hub, p, open }
    }

    fn calls(p: &MockProvider) -> usize {
        p.quote_calls.load(Ordering::SeqCst)
    }

    async fn sleep(secs: u64) {
        tokio::time::sleep(Duration::from_secs(secs)).await;
    }

    fn drain(rx: &mut mpsc::UnboundedReceiver<HubUpdate>) -> Vec<HubUpdate> {
        let mut out = Vec::new();
        while let Ok(u) = rx.try_recv() {
            out.push(u);
        }
        out
    }

    #[tokio::test(start_paused = true)]
    async fn a_pinned_topic_asks_only_its_own_provider() {
        let nse = Arc::new(MockProvider::new(ProviderId::Nse, Capabilities::QUOTE));
        let yahoo = Arc::new(MockProvider::new(ProviderId::Yahoo, Capabilities::QUOTE));
        nse.repeat_quotes(Ok(vec![quote("TCS")]));
        let mut y = quote("TCS");
        y.provenance.provider = ProviderId::Yahoo;
        yahoo.repeat_quotes(Ok(vec![y]));
        let md = MarketData::builder()
            .provider(nse.clone())
            .provider(yahoo.clone())
            .build();
        let hub = QuoteHub::spawn(md, HubConfig::default(), || true);
        let (tx, mut rx) = mpsc::unbounded_channel();
        hub.subscribe([Topic::Pinned(ProviderId::Yahoo, id("TCS"))], tx);
        sleep(1).await;
        let got = drain(&mut rx);
        assert!(
            matches!(&got[..], [HubUpdate::Pinned(q)] if q.provenance.provider == ProviderId::Yahoo),
            "{got:?}"
        );
        // NSE is first in the default routing but was never asked.
        assert_eq!(calls(&nse), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn nothing_is_polled_without_subscribers() {
        let r = rig();
        sleep(120).await;
        assert_eq!(calls(&r.p), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_subscription_gets_an_immediate_poll_then_one_per_interval() {
        let r = rig();
        let (tx, mut rx) = mpsc::unbounded_channel();
        r.hub.subscribe([Topic::Quote(id("TCS"))], tx);
        sleep(1).await;
        assert_eq!(calls(&r.p), 1);
        let first = drain(&mut rx);
        assert_eq!(first.len(), 1);
        assert!(
            matches!(&first[0], HubUpdate::Quotes(q) if q.len() == 1 && q[0].data.id == id("TCS"))
        );
        sleep(30).await; // open market: ~every 5 s
        let n = calls(&r.p);
        assert!((6..=8).contains(&n), "{n} polls in 31 s");
    }

    #[tokio::test(start_paused = true)]
    async fn all_subscribers_share_one_batched_call() {
        let r = rig();
        let (a_tx, mut a_rx) = mpsc::unbounded_channel();
        let (b_tx, mut b_rx) = mpsc::unbounded_channel();
        r.hub.subscribe([Topic::Quote(id("TCS"))], a_tx);
        r.hub.subscribe([Topic::Quote(id("INFY"))], b_tx);
        sleep(1).await;
        assert_eq!(calls(&r.p), 1, "one provider call for both views");
        let mut asked = r.p.quote_requests()[0].clone();
        asked.sort();
        assert_eq!(asked, [id("INFY"), id("TCS")]);
        // Each view only receives its own symbol.
        let a = drain(&mut a_rx);
        let b = drain(&mut b_rx);
        assert!(
            matches!(&a[..], [HubUpdate::Quotes(q)] if q.len() == 1 && q[0].data.id == id("TCS"))
        );
        assert!(
            matches!(&b[..], [HubUpdate::Quotes(q)] if q.len() == 1 && q[0].data.id == id("INFY"))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn subscriptions_are_reference_counted_per_topic() {
        let r = rig();
        let (tx1, _rx1) = mpsc::unbounded_channel();
        let (tx2, _rx2) = mpsc::unbounded_channel();
        let s1 = r.hub.subscribe([Topic::Quote(id("TCS"))], tx1);
        let s2 = r.hub.subscribe([Topic::Quote(id("TCS"))], tx2);
        assert_eq!((r.hub.topic_count(), r.hub.subscription_count()), (1, 2));
        sleep(1).await;
        assert_eq!(
            r.p.quote_requests()[0],
            [id("TCS")],
            "asked for once, not twice"
        );

        r.hub.unsubscribe(s1);
        assert_eq!(r.hub.topic_count(), 1, "still needed by the second view");
        let before = calls(&r.p);
        sleep(11).await;
        assert!(calls(&r.p) > before, "still polling");

        r.hub.unsubscribe(s2);
        assert_eq!(r.hub.topic_count(), 0);
        sleep(2).await;
        let settled = calls(&r.p);
        sleep(600).await;
        assert_eq!(
            calls(&r.p),
            settled,
            "polling stops with the last subscriber"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn polling_slows_down_when_the_market_is_closed() {
        let r = rig();
        r.open.store(false, Ordering::SeqCst);
        let (tx, _rx) = mpsc::unbounded_channel();
        r.hub.subscribe([Topic::Quote(id("TCS"))], tx);
        sleep(1).await;
        let start = calls(&r.p);
        sleep(300).await;
        let n = calls(&r.p) - start;
        assert!((4..=6).contains(&n), "{n} polls in 5 minutes at 60 s");
    }

    #[tokio::test(start_paused = true)]
    async fn a_hidden_window_stops_polling_and_showing_it_polls_at_once() {
        let r = rig();
        let (tx, _rx) = mpsc::unbounded_channel();
        r.hub.subscribe([Topic::Quote(id("TCS"))], tx);
        sleep(1).await;
        r.hub.set_visible(false);
        sleep(7).await; // let any in-flight poll finish
        let paused_at = calls(&r.p);
        sleep(900).await;
        assert_eq!(calls(&r.p), paused_at, "no polling while hidden");
        r.hub.set_visible(true);
        sleep(1).await;
        assert_eq!(calls(&r.p), paused_at + 1, "immediate poll on show");
    }

    #[tokio::test(start_paused = true)]
    async fn a_dropped_receiver_ends_its_subscription() {
        let r = rig();
        let (tx, rx) = mpsc::unbounded_channel();
        r.hub.subscribe([Topic::Quote(id("TCS"))], tx);
        sleep(1).await;
        drop(rx); // the webview closed the channel
        sleep(11).await;
        assert_eq!((r.hub.subscription_count(), r.hub.topic_count()), (0, 0));
        let settled = calls(&r.p);
        sleep(300).await;
        assert_eq!(calls(&r.p), settled);
    }

    #[tokio::test(start_paused = true)]
    async fn failures_reach_subscribers_and_polling_continues() {
        let r = rig();
        r.p.repeat_quotes(Err(DataError::Upstream("offline".into())));
        let (tx, mut rx) = mpsc::unbounded_channel();
        r.hub.subscribe([Topic::Quote(id("TCS"))], tx);
        sleep(1).await;
        let got = drain(&mut rx);
        assert!(
            matches!(&got[..], [HubUpdate::Failed { topic, .. }] if *topic == Topic::Quote(id("TCS")))
        );
        // Recovery: the next poll delivers data.
        r.p.repeat_quotes(Ok(vec![quote("TCS")]));
        sleep(6).await;
        assert!(
            drain(&mut rx)
                .iter()
                .any(|u| matches!(u, HubUpdate::Quotes(_)))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_late_subscriber_is_shown_the_latest_data_at_once() {
        let r = rig();
        let (tx, _rx) = mpsc::unbounded_channel();
        r.hub.subscribe([Topic::Quote(id("TCS"))], tx);
        sleep(1).await;
        let (tx2, mut rx2) = mpsc::unbounded_channel();
        r.hub.subscribe([Topic::Quote(id("TCS"))], tx2);
        // Replayed synchronously, before any new poll.
        let replay = drain(&mut rx2);
        assert!(matches!(&replay[..], [HubUpdate::Quotes(q)] if q[0].data.id == id("TCS")));
    }

    #[tokio::test(start_paused = true)]
    async fn index_and_dashboard_topics_are_polled_too() {
        use nivesh_core::{IndexQuote, IndexSnapshot};
        let r = rig();
        let now = Utc::now();
        let prov = Provenance::new(ProviderId::Nse, FeedKind::Delayed, now, now);
        let level = IndexQuote {
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
        };
        r.p.repeat_index(Ok(Sourced::new(
            IndexSnapshot {
                quote: level.clone(),
                constituents: vec![quote("TCS").data],
                constituents_info: None,
            },
            prov.clone(),
        )));
        r.p.repeat_all_indices(Ok(vec![Sourced::new(level, prov)]));
        let (tx, mut rx) = mpsc::unbounded_channel();
        r.hub
            .subscribe([Topic::Index(IndexId::Nifty50), Topic::AllIndices], tx);
        sleep(1).await;
        let got = drain(&mut rx);
        assert!(got.iter().any(|u| matches!(u, HubUpdate::Index(_))));
        assert!(got.iter().any(|u| matches!(u, HubUpdate::AllIndices(_))));
        assert_eq!(calls(&r.p), 0, "no stock topics, so no stock quote calls");
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_stops_the_poller() {
        let r = rig();
        let (tx, _rx) = mpsc::unbounded_channel();
        r.hub.subscribe([Topic::Quote(id("TCS"))], tx);
        sleep(1).await;
        r.hub.shutdown();
        sleep(1).await;
        let at = calls(&r.p);
        sleep(600).await;
        assert_eq!(calls(&r.p), at);
    }
}
