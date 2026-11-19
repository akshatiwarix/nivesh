//! Paper trading in the app: wiring only. The decisions live in
//! `nivesh-exec::PaperDesk`; this module feeds it closed bars and persists
//! what it reports.
//!
//! One task per running strategy subscribes to the quote hub, pinned to the
//! provider its history came from, and builds bars with
//! [`FormingBar::apply_closing`]. Each closed bar goes through the desk
//! (gateway, risk check, paper fill at the next open), and the strategy's
//! state is saved after every bar. At startup, every strategy saved as
//! running resumes from that state. Nothing here can reach a real broker.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nivesh_core::{Candle, FeedKind, Sourced};
use nivesh_data::forming::FormingBar;
use nivesh_data::{HubUpdate, QuoteHub, SubscriptionId, Topic};
use nivesh_exec::PaperDesk;
use nivesh_risk::{RiskEngine, RiskLimits};
use nivesh_store::Store;
use nivesh_strategy::{PaperRunner, PaperState, RuleSet};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::commands::chart::parse_request;
use crate::ipc::IpcError;
use crate::services::Services;

/// Starting paper equity, rupees (sizing and the equity mark).
pub const PAPER_EQUITY: f64 = 100_000.0;

/// What the store keeps per paper strategy: its stream and its state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PaperRecord {
    pub exchange: String,
    pub symbol: String,
    pub interval: String,
    /// Whether the bars came from the broker's real-time feed. Anything
    /// else is DELAYED, and paper fills on it are approximate.
    pub live_feed: bool,
    #[serde(flatten)]
    pub state: PaperState,
}

impl Default for PaperRecord {
    /// Part 5's first cut stored only `{"day_pnl":0.0}` and always meant
    /// RELIANCE daily bars.
    fn default() -> Self {
        Self {
            exchange: "NSE".into(),
            symbol: "RELIANCE".into(),
            interval: "1D".into(),
            live_feed: false,
            state: PaperState::default(),
        }
    }
}

impl PaperRecord {
    pub fn decode(json: &str) -> Self {
        serde_json::from_str(json).unwrap_or_default()
    }

    fn encode(&self) -> Result<String, IpcError> {
        serde_json::to_string(self)
            .map_err(|e| IpcError::internal(format!("cannot encode the paper state: {e}")))
    }

    fn same_stream(&self, exchange: &str, symbol: &str, interval: &str) -> bool {
        self.exchange.eq_ignore_ascii_case(exchange)
            && self.symbol.eq_ignore_ascii_case(symbol)
            && self.interval.eq_ignore_ascii_case(interval)
    }
}

struct Running {
    task: tauri::async_runtime::JoinHandle<()>,
    subscription: SubscriptionId,
}

pub struct PaperManager {
    store: Arc<Store>,
    desk: Arc<Mutex<PaperDesk>>,
    running: Mutex<HashMap<i64, Running>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn now_ms() -> i64 {
    nivesh_risk::now_ms()
}

impl PaperManager {
    pub fn new(store: Arc<Store>) -> Self {
        let audit = Arc::new(crate::audit::StoreAudit(Arc::clone(&store)));
        Self {
            store,
            desk: Arc::new(Mutex::new(PaperDesk::new(
                RiskEngine::new(RiskLimits::default()),
                audit,
            ))),
            running: Mutex::new(HashMap::new()),
        }
    }

    /// Starts (or restarts) a strategy on a stream. The saved state resumes
    /// when the stream is the same; a different stream starts flat, and is
    /// refused while the old one still holds a position.
    pub async fn start(
        &self,
        services: &Arc<Services>,
        strategy_id: i64,
        set: RuleSet,
        exchange: &str,
        symbol: &str,
        interval: &str,
    ) -> Result<PaperRecord, IpcError> {
        let req = parse_request(exchange, symbol, interval, None)?;
        let saved = self
            .store
            .paper_state(strategy_id)?
            .map(|(_, json)| PaperRecord::decode(&json));
        let state = match saved {
            Some(rec) if rec.same_stream(exchange, symbol, interval) => rec.state,
            Some(rec) if rec.state.position.is_some() => {
                return Err(IpcError::invalid_argument(format!(
                    "this strategy still holds a paper position in {}:{}; \
                     restart it on that stream, or stop it and start again",
                    rec.exchange, rec.symbol
                )));
            }
            _ => PaperState::default(),
        };

        // Warm the indicators on history; its last bar may still be forming,
        // so it becomes the forming bar instead.
        let series = services.market.history(&req).await?;
        let candles = &series.data.candles;
        let (closed, forming) = match candles.split_last() {
            Some((last, closed)) => (closed, Some(*last)),
            None => (&candles[..], None),
        };
        let mut runner = PaperRunner::new(set, req.id.to_string(), PAPER_EQUITY)
            .map_err(|e| IpcError::invalid_argument(e.to_string()))?
            .with_state(state);
        runner.warm_up(closed);

        let record = PaperRecord {
            exchange: req.id.exchange().to_string(),
            symbol: req.id.symbol().to_owned(),
            interval: req.interval.to_string(),
            live_feed: series.provenance.feed == FeedKind::Live,
            state: runner.state().clone(),
        };
        self.stop_task(strategy_id);
        lock(&self.desk).start(strategy_id, runner);
        self.store
            .save_paper_state(strategy_id, true, &record.encode()?)?;

        let (tx, rx) = mpsc::unbounded_channel();
        let subscription = services.hub.subscribe(
            vec![Topic::Pinned(series.provenance.provider, req.id.clone())],
            tx,
        );
        let task = tauri::async_runtime::spawn(feed(
            strategy_id,
            FormingBar::new(req.interval, forming),
            rx,
            Arc::clone(&self.desk),
            Arc::clone(&self.store),
            record.clone(),
            services.hub.clone(),
            subscription,
        ));
        lock(&self.running).insert(strategy_id, Running { task, subscription });
        tracing::info!(strategy_id, stream = %req.id, interval = %req.interval, "paper strategy started");
        Ok(record)
    }

    /// Stops a strategy and keeps its state, marked stopped.
    pub fn stop(&self, services: &Services, strategy_id: i64) -> Result<(), IpcError> {
        if let Some(running) = lock(&self.running).remove(&strategy_id) {
            running.task.abort();
            services.hub.unsubscribe(running.subscription);
        }
        let state = lock(&self.desk).stop(strategy_id);
        let mut record = self
            .store
            .paper_state(strategy_id)?
            .map(|(_, json)| PaperRecord::decode(&json))
            .unwrap_or_default();
        if let Some(state) = state {
            record.state = state;
        }
        self.store
            .save_paper_state(strategy_id, false, &record.encode()?)?;
        Ok(())
    }

    /// Resumes every strategy saved as running. Failures are logged and
    /// the strategy stays marked running, to retry at the next start.
    pub async fn resume_all(&self, services: &Arc<Services>) {
        let rows = match self.store.strategies() {
            Ok(rows) => rows,
            Err(err) => {
                tracing::warn!(error = %err, "listing strategies to resume failed");
                return;
            }
        };
        for row in rows {
            let Ok(Some((true, json))) = self.store.paper_state(row.id) else {
                continue;
            };
            let record = PaperRecord::decode(&json);
            let set: RuleSet = match serde_json::from_str(&row.json) {
                Ok(set) => set,
                Err(err) => {
                    tracing::warn!(strategy_id = row.id, error = %err, "unreadable strategy; not resumed");
                    continue;
                }
            };
            if let Err(err) = self
                .start(
                    services,
                    row.id,
                    set,
                    &record.exchange,
                    &record.symbol,
                    &record.interval,
                )
                .await
            {
                tracing::warn!(strategy_id = row.id, error = ?err, "resuming paper strategy failed");
            }
        }
    }

    pub fn is_running(&self, strategy_id: i64) -> bool {
        lock(&self.desk).is_running(strategy_id)
    }

    /// The live state of a running strategy.
    pub fn state(&self, strategy_id: i64) -> Option<PaperState> {
        lock(&self.desk).state(strategy_id).cloned()
    }

    fn stop_task(&self, strategy_id: i64) {
        if let Some(running) = lock(&self.running).remove(&strategy_id) {
            running.task.abort();
        }
    }
}

/// The per-strategy loop: prices in, closed bars through the desk, state
/// saved after every bar.
#[allow(clippy::too_many_arguments)]
async fn feed(
    strategy_id: i64,
    mut forming: FormingBar,
    mut rx: mpsc::UnboundedReceiver<HubUpdate>,
    desk: Arc<Mutex<PaperDesk>>,
    store: Arc<Store>,
    mut record: PaperRecord,
    hub: QuoteHub,
    subscription: SubscriptionId,
) {
    while let Some(update) = rx.recv().await {
        let HubUpdate::Pinned(Sourced { data, provenance }) = update else {
            continue;
        };
        let Some(closed) = forming.apply_closing(data.ltp, provenance.as_of.timestamp_millis())
        else {
            continue;
        };
        if !on_closed_bar(strategy_id, closed, &desk, &store, &mut record) {
            break;
        }
    }
    hub.unsubscribe(subscription);
}

/// Returns `false` when the strategy is no longer running.
fn on_closed_bar(
    strategy_id: i64,
    bar: Candle,
    desk: &Mutex<PaperDesk>,
    store: &Store,
    record: &mut PaperRecord,
) -> bool {
    let Some(report) = lock(desk).on_closed_bar(strategy_id, bar, now_ms()) else {
        return false;
    };
    for fill in &report.fills {
        tracing::info!(strategy_id, side = ?fill.side, quantity = fill.quantity, price = fill.price, "paper fill");
    }
    for refusal in &report.refused {
        tracing::info!(strategy_id, reason = %refusal, "paper order refused");
    }
    record.state = report.state;
    match record.encode() {
        Ok(json) => {
            if let Err(err) = store.save_paper_state(strategy_id, true, &json) {
                tracing::warn!(strategy_id, error = %err, "saving paper state failed");
            }
        }
        Err(err) => tracing::warn!(strategy_id, error = ?err, "encoding paper state failed"),
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_record_decodes_with_the_v1_stream() {
        let rec = PaperRecord::decode(r#"{"day_pnl":12.5}"#);
        assert_eq!(
            (rec.exchange.as_str(), rec.symbol.as_str()),
            ("NSE", "RELIANCE")
        );
        assert_eq!(rec.interval, "1D");
        assert_eq!(rec.state.day_pnl, 12.5);
        assert!(!rec.live_feed);
    }

    #[test]
    fn a_record_round_trips_flat() {
        let rec = PaperRecord {
            exchange: "NSE".into(),
            symbol: "TCS".into(),
            interval: "5m".into(),
            live_feed: true,
            state: PaperState {
                realised_pnl: 3.0,
                ..PaperState::default()
            },
        };
        let json = rec.encode().unwrap();
        assert!(
            json.contains(r#""realised_pnl":3.0"#),
            "state is flattened: {json}"
        );
        assert_eq!(PaperRecord::decode(&json), rec);
        assert!(rec.same_stream("nse", "tcs", "5M"));
        assert!(!rec.same_stream("NSE", "TCS", "1D"));
    }

    #[test]
    fn closed_bars_drive_the_desk_and_the_state_is_saved() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let id = store
            .save_strategy(None, "always", "test", "{}", 0)
            .unwrap();
        let set = RuleSet {
            entry: nivesh_strategy::Rule::True,
            sizing: nivesh_strategy::Sizing::FixedQuantity(1),
            ..RuleSet::default()
        };
        // The test bars are stamped with the wall clock: no trading window.
        let desk = Mutex::new(PaperDesk::new(
            RiskEngine::new(RiskLimits {
                trading_window: None,
                ..RiskLimits::default()
            }),
            Arc::new(crate::audit::StoreAudit(Arc::clone(&store))),
        ));
        lock(&desk).start(id, PaperRunner::new(set, "NSE:TCS", PAPER_EQUITY).unwrap());
        let mut record = PaperRecord::default();
        let t0 = now_ms();
        let bar = |t: i64, o: f64| Candle {
            t,
            o,
            h: o,
            l: o,
            c: o,
            v: 0.0,
        };
        assert!(on_closed_bar(
            id,
            bar(t0, 100.0),
            &desk,
            &store,
            &mut record
        ));
        assert!(on_closed_bar(
            id,
            bar(t0 + 60_000, 101.0),
            &desk,
            &store,
            &mut record
        ));

        let (running, json) = store.paper_state(id).unwrap().unwrap();
        assert!(running);
        let saved = PaperRecord::decode(&json);
        let pos = saved
            .state
            .position
            .expect("filled at the second bar's open");
        // The default cost model: slippage on the price, charges on the fill.
        let costs = nivesh_strategy::CostModel::default();
        let price = costs.fill_price(nivesh_core::Side::Buy, 101.0);
        assert_eq!((pos.quantity, pos.entry_price), (1, price));
        assert_eq!(
            saved.state.charges,
            costs
                .charges(nivesh_core::Side::Buy, 1, price)
                .total_rupees()
        );

        // Every step went through the audit chain first.
        let kinds: Vec<String> = store
            .audit(&nivesh_store::AuditQuery::default())
            .unwrap()
            .into_iter()
            .rev()
            .map(|r| r.kind)
            .collect();
        assert_eq!(kinds, ["intent", "order.routed", "fill"]);
        assert!(store.verify_audit().unwrap().is_intact());

        lock(&desk).stop(id);
        assert!(!on_closed_bar(
            id,
            bar(t0 + 120_000, 102.0),
            &desk,
            &store,
            &mut record
        ));
    }
}
