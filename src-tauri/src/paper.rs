//! Paper trading in the app: wiring only. The decisions live in
//! `nivesh-exec::PaperDesk`; this module feeds it closed bars and persists
//! what it reports.
//!
//! One task per running strategy subscribes to the quote hub, pinned to the
//! provider its history came from, and builds bars with
//! [`FormingBar::apply_closing`]. Each closed bar goes through the desk
//! (gateway, risk check, paper fill at the next open), and the strategy's
//! state is saved after every bar. At startup, every strategy saved as
//! running resumes from that state.
//!
//! The safety controls live here too: limits (saved, changes audited),
//! the session-only master switch, arming and the kill switch, each change
//! reported to the UI as `killswitch:changed` / `risk:alert`. No live
//! route is installed yet, so nothing can be armed and nothing here can
//! reach a real broker.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nivesh_core::{Candle, FeedKind, Freshness, Sourced};
use nivesh_data::forming::FormingBar;
use nivesh_data::{HubUpdate, QuoteHub, SubscriptionId, Topic};
use nivesh_exec::{
    ArmConditions, ArmError, DeskUsage, GatewayError, KillReason, KillState, LiveDispatch,
    PaperDesk,
};
use nivesh_risk::{RiskEngine, RiskLimits};
use nivesh_store::Store;
use nivesh_strategy::{PaperRunner, PaperState, RuleSet};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::commands::chart::parse_request;
use crate::ipc::IpcError;
use crate::ipc::risk::AlertLevel;
use crate::notify::Notifier;
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
    notifier: Notifier,
}

/// The outcome of an action that may need a typed confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confirm {
    Done,
    /// Nothing changed: type this phrase and try again.
    Needs(String),
}

/// A strategy's safety state, for the RISK view.
#[derive(Debug, Clone, PartialEq)]
pub struct StrategyRisk {
    pub id: i64,
    pub running: bool,
    pub armed_until: Option<i64>,
    pub paused: Option<String>,
    pub limits: Option<RiskLimits>,
}

/// Everything the RISK view shows, at one instant.
#[derive(Debug, Clone, PartialEq)]
pub struct RiskView {
    pub kill: KillState,
    pub live_enabled: bool,
    pub live_route: bool,
    pub limits: RiskLimits,
    pub usage: DeskUsage,
    pub strategies: Vec<StrategyRisk>,
}

/// A refusal that only asks for the phrase becomes [`Confirm::Needs`];
/// anything else is an error for the user.
fn confirmable(result: Result<(), GatewayError>) -> Result<Confirm, IpcError> {
    match result {
        Ok(()) => Ok(Confirm::Done),
        Err(GatewayError::NeedsConfirmation { phrase })
        | Err(GatewayError::Arm(ArmError::WrongPhrase { expected: phrase })) => {
            Ok(Confirm::Needs(phrase))
        }
        Err(GatewayError::AuditFailed(e)) => Err(IpcError::internal(format!(
            "the audit log could not be written, so nothing changed: {e}"
        ))),
        Err(err) => Err(IpcError::invalid_argument(err.to_string())),
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn now_ms() -> i64 {
    nivesh_risk::now_ms()
}

impl PaperManager {
    /// A desk over the store, with the kill switch as it was saved: an
    /// engaged switch stays engaged across a restart.
    ///
    /// The saved limits come back too. Unreadable global limits fall back
    /// to the conservative defaults with the kill switch engaged (the saved
    /// ones may have been tighter); an unreadable strategy's limits are
    /// dropped, which leaves it unable to arm.
    pub fn new(store: Arc<Store>) -> Self {
        let audit = Arc::new(crate::audit::StoreAudit(Arc::clone(&store)));
        let (limits, limits_ok) = load_limits(&store);
        let mut desk = PaperDesk::new(RiskEngine::new(limits), audit);
        let mut kill = load_kill(&store);
        if !limits_ok && !kill.engaged {
            kill = unreadable(SAVED_LIMITS_UNREADABLE);
        }
        desk.restore_kill(kill);
        for row in store.strategies().unwrap_or_default() {
            desk.set_strategy_limits(row.id, load_strategy_limits(&store, row.id));
        }
        Self {
            store,
            desk: Arc::new(Mutex::new(desk)),
            running: Mutex::new(HashMap::new()),
            notifier: Notifier::default(),
        }
    }

    /// Lets the manager emit its events once the app is up.
    pub fn attach(&self, app: tauri::AppHandle) {
        self.notifier.attach(app);
    }

    /// The safety state the RISK view shows. `ids`: every saved strategy.
    pub fn risk_view(&self, ids: &[i64]) -> RiskView {
        let now = now_ms();
        let desk = lock(&self.desk);
        RiskView {
            kill: desk.kill_state().clone(),
            live_enabled: desk.live_enabled(),
            live_route: desk.live_route().is_some(),
            limits: desk.risk_limits().clone(),
            usage: desk.usage(now),
            strategies: ids
                .iter()
                .map(|&id| StrategyRisk {
                    id,
                    running: desk.is_running(id),
                    armed_until: desk.armed_until(id, now),
                    paused: desk.paused(id).map(str::to_owned),
                    limits: desk.strategy_limits(id).cloned(),
                })
                .collect(),
        }
    }

    /// Changes the global limits (`strategy_id` none) or a strategy's own
    /// (`edit` starts from its current ones, or a copy of the global ones).
    /// Loosening needs the typed phrase. Saved once applied.
    pub fn change_limits(
        &self,
        strategy_id: Option<i64>,
        edit: impl FnOnce(&mut RiskLimits),
        typed: &str,
    ) -> Result<Confirm, IpcError> {
        let mut desk = lock(&self.desk);
        let now = now_ms();
        match strategy_id {
            None => {
                let mut limits = desk.risk_limits().clone();
                edit(&mut limits);
                let json = encode_limits(&limits)?;
                let done = confirmable(desk.change_risk_limits(limits, typed, now).map(drop))?;
                if done == Confirm::Done {
                    self.store.set_setting(LIMITS_SETTING, &json)?;
                }
                Ok(done)
            }
            Some(id) => {
                let mut limits = desk
                    .strategy_limits(id)
                    .unwrap_or_else(|| desk.risk_limits())
                    .clone();
                edit(&mut limits);
                let json = encode_limits(&limits)?;
                let done = confirmable(
                    desk.change_strategy_limits(id, Some(limits), typed, now)
                        .map(drop),
                )?;
                if done == Confirm::Done {
                    self.store.set_setting(&strategy_limits_key(id), &json)?;
                }
                Ok(done)
            }
        }
    }

    /// Removes a strategy's own limits (a loosening: typed phrase).
    pub fn clear_strategy_limits(
        &self,
        strategy_id: i64,
        typed: &str,
    ) -> Result<Confirm, IpcError> {
        let mut desk = lock(&self.desk);
        let done = confirmable(
            desk.change_strategy_limits(strategy_id, None, typed, now_ms())
                .map(drop),
        )?;
        if done == Confirm::Done {
            // "null" reads back as no limits.
            self.store
                .set_setting(&strategy_limits_key(strategy_id), "null")?;
        }
        Ok(done)
    }

    /// The master switch: on needs the typed phrase; off is immediate and
    /// disarms everything. Never saved: every start is off.
    pub fn set_live_enabled(&self, on: bool, typed: &str) -> Result<Confirm, IpcError> {
        confirmable(lock(&self.desk).set_live_enabled(on, typed, now_ms()))
    }

    /// Arms a running strategy (every precondition, then the phrase).
    pub fn arm(
        &self,
        strategy_id: i64,
        name: &str,
        typed: &str,
        broker_connected: bool,
    ) -> Result<Confirm, IpcError> {
        let live_feed = self
            .store
            .paper_state(strategy_id)?
            .is_some_and(|(_, json)| PaperRecord::decode(&json).live_feed);
        let conditions = ArmConditions {
            broker_connected,
            data: if live_feed {
                Freshness::Live
            } else {
                Freshness::Delayed
            },
        };
        let result = lock(&self.desk).arm(strategy_id, name, typed, conditions, now_ms());
        confirmable(result.map(drop))
    }

    /// Disarms a strategy at once.
    pub fn disarm(&self, strategy_id: i64) -> bool {
        lock(&self.desk).disarm(strategy_id, now_ms())
    }

    pub fn kill_state(&self) -> KillState {
        lock(&self.desk).kill_state().clone()
    }

    /// Engages the kill switch now and saves it. Returns how many open
    /// orders it cancelled (`None` if it was already engaged).
    pub fn engage_kill(&self, detail: &str) -> Result<Option<usize>, IpcError> {
        let mut desk = lock(&self.desk);
        let cancelled = desk.engage_kill(KillReason::Manual, detail, now_ms());
        if cancelled.is_some() {
            self.notifier.kill_changed(desk.kill_state());
        }
        save_kill(&self.store, desk.kill_state())?;
        drop(desk);
        cancel_at_broker(&self.desk, &self.notifier);
        Ok(cancelled.map(|c| c.len()))
    }

    /// Resets the kill switch with the typed phrase (after the cool-down)
    /// and saves it.
    pub fn reset_kill(&self, typed: &str) -> Result<(), IpcError> {
        let mut desk = lock(&self.desk);
        desk.reset_kill(typed, now_ms())
            .map_err(|e| IpcError::invalid_argument(e.to_string()))?;
        self.notifier.kill_changed(desk.kill_state());
        save_kill(&self.store, desk.kill_state())
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
        {
            let mut desk = lock(&self.desk);
            desk.start(strategy_id, runner, now_ms());
            desk.set_strategy_limits(strategy_id, load_strategy_limits(&self.store, strategy_id));
        }
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
            self.notifier.clone(),
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
        let state = lock(&self.desk).stop(strategy_id, now_ms());
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

/// Where the kill switch's state lives.
const KILL_SETTING: &str = "kill_switch";

/// The saved kill switch. Unreadable state fails closed: engaged.
fn load_kill(store: &Store) -> KillState {
    match store.setting(KILL_SETTING) {
        Ok(None) => KillState::default(),
        Ok(Some(json)) => serde_json::from_str(&json).unwrap_or_else(|_| unreadable_kill()),
        Err(_) => unreadable_kill(),
    }
}

fn unreadable_kill() -> KillState {
    unreadable("the saved kill-switch state could not be read")
}

const SAVED_LIMITS_UNREADABLE: &str =
    "the saved risk limits could not be read; the defaults apply until you review them";

fn unreadable(detail: &str) -> KillState {
    KillState {
        engaged: true,
        reason: Some(KillReason::Manual),
        detail: detail.into(),
        engaged_at_ms: Some(now_ms()),
    }
}

/// Where the global limits live; a strategy's are under
/// [`strategy_limits_key`].
const LIMITS_SETTING: &str = "risk_limits";

fn strategy_limits_key(strategy_id: i64) -> String {
    format!("risk_limits:{strategy_id}")
}

fn encode_limits(limits: &RiskLimits) -> Result<String, IpcError> {
    serde_json::to_string(limits)
        .map_err(|e| IpcError::internal(format!("cannot encode the limits: {e}")))
}

/// The saved global limits, and whether they were readable.
fn load_limits(store: &Store) -> (RiskLimits, bool) {
    match store.setting(LIMITS_SETTING) {
        Ok(None) => (RiskLimits::default(), true),
        Ok(Some(json)) => match serde_json::from_str(&json) {
            Ok(limits) => (limits, true),
            Err(_) => (RiskLimits::default(), false),
        },
        Err(_) => (RiskLimits::default(), false),
    }
}

/// A strategy's saved limits; unreadable ones are dropped (fail closed: a
/// strategy without limits cannot arm).
fn load_strategy_limits(store: &Store, strategy_id: i64) -> Option<RiskLimits> {
    let json = store.setting(&strategy_limits_key(strategy_id)).ok()??;
    serde_json::from_str::<Option<RiskLimits>>(&json).unwrap_or_else(|_| {
        tracing::warn!(strategy_id, "unreadable strategy limits dropped");
        None
    })
}

fn save_kill(store: &Store, state: &KillState) -> Result<(), IpcError> {
    let json = serde_json::to_string(state)
        .map_err(|e| IpcError::internal(format!("cannot encode the kill switch: {e}")))?;
    store.set_setting(KILL_SETTING, &json)?;
    Ok(())
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
    notifier: Notifier,
) {
    while let Some(update) = rx.recv().await {
        let HubUpdate::Pinned(Sourced { data, provenance }) = update else {
            continue;
        };
        let Some(closed) = forming.apply_closing(data.ltp, provenance.as_of.timestamp_millis())
        else {
            continue;
        };
        let freshness = provenance.freshness();
        let Some(dispatches) = on_closed_bar(
            strategy_id,
            closed,
            freshness,
            &desk,
            &store,
            &mut record,
            &notifier,
        ) else {
            break;
        };
        // Live orders (armed strategies only) are sent outside the lock.
        for dispatch in dispatches {
            let outcome = dispatch.send(now_ms()).await;
            if let Err(err) = &outcome {
                let level = match err {
                    GatewayError::AmbiguousPlacement(_) | GatewayError::AuditFailed(_) => {
                        AlertLevel::Critical
                    }
                    _ => AlertLevel::Warn,
                };
                notifier.alert(level, Some(strategy_id), format!("live order: {err}"));
            }
            let kill = lock(&desk).on_live_outcome(strategy_id, &outcome, now_ms());
            if let Some(kill) = kill {
                tripped(&desk, &notifier, &store, &kill);
            }
        }
    }
    hub.unsubscribe(subscription);
}

/// The kill switch's broker half: with a live route, every open
/// Nivesh-tagged order there is cancelled, in the background (blocking new
/// orders already happened and never waits on the network).
fn cancel_at_broker(desk: &Mutex<PaperDesk>, notifier: &Notifier) {
    let Some(route) = lock(desk).live_route().cloned() else {
        return;
    };
    let notifier = notifier.clone();
    tauri::async_runtime::spawn(async move {
        match route.cancel_open(now_ms()).await {
            Ok(report) if report.failed.is_empty() => {
                tracing::info!(cancelled = report.cancelled.len(), "kill switch: broker orders cancelled");
            }
            Ok(report) => notifier.alert(
                AlertLevel::Critical,
                None,
                format!(
                    "kill switch: {} order(s) could not be cancelled at the broker: {}",
                    report.failed.len(),
                    report
                        .failed
                        .iter()
                        .map(|(tag, why)| format!("{tag} ({why})"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ),
            Err(err) => notifier.alert(
                AlertLevel::Critical,
                None,
                format!("kill switch: the broker's order book could not be read, so its open orders were not cancelled: {err}"),
            ),
        }
    });
}

/// The kill switch tripped on its own: tell the UI, save it, and cancel at
/// the broker.
fn tripped(desk: &Mutex<PaperDesk>, notifier: &Notifier, store: &Store, kill: &KillState) {
    cancel_at_broker(desk, notifier);
    notifier.kill_changed(kill);
    notifier.alert(
        AlertLevel::Critical,
        None,
        format!("kill switch engaged: {}", kill.detail),
    );
    if let Err(err) = save_kill(store, kill) {
        tracing::error!(error = ?err, "saving the kill switch failed");
    }
}

/// Returns `None` when the strategy is no longer running, else the live
/// orders to send.
fn on_closed_bar(
    strategy_id: i64,
    bar: Candle,
    freshness: Freshness,
    desk: &Mutex<PaperDesk>,
    store: &Store,
    record: &mut PaperRecord,
    notifier: &Notifier,
) -> Option<Vec<LiveDispatch>> {
    let report = lock(desk).on_closed_bar(strategy_id, bar, freshness, now_ms())?;
    for fill in &report.fills {
        tracing::info!(strategy_id, side = ?fill.side, quantity = fill.quantity, price = fill.price, "paper fill");
    }
    for refusal in &report.refused {
        tracing::info!(strategy_id, reason = %refusal, "paper order refused");
        if matches!(refusal, GatewayError::NotLiveData(_)) {
            notifier.alert(AlertLevel::Warn, Some(strategy_id), refusal.to_string());
        }
    }
    if let Some(kill) = &report.kill {
        tripped(desk, notifier, store, kill);
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
    Some(report.dispatches)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kill_switch_is_saved_and_survives_a_restart() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let manager = PaperManager::new(Arc::clone(&store));
        assert!(!manager.kill_state().engaged);
        assert_eq!(manager.engage_kill("test").unwrap(), Some(0));
        assert_eq!(
            manager.engage_kill("again").unwrap(),
            None,
            "already engaged"
        );
        // A restart: a new manager over the same store.
        let restarted = PaperManager::new(Arc::clone(&store));
        let state = restarted.kill_state();
        assert!(state.engaged);
        assert_eq!(state.reason, Some(KillReason::Manual));
        assert!(
            restarted.reset_kill(nivesh_exec::RESET_PHRASE).is_err(),
            "cool-down"
        );
        let kinds: Vec<String> = store
            .audit(&nivesh_store::AuditQuery::default())
            .unwrap()
            .into_iter()
            .map(|r| r.kind)
            .collect();
        assert_eq!(kinds, ["kill.engaged"]);
    }

    #[test]
    fn limits_are_saved_and_loosening_needs_the_phrase() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let manager = PaperManager::new(Arc::clone(&store));
        let tighten = |l: &mut RiskLimits| l.max_order_quantity = 10;
        assert_eq!(
            manager.change_limits(None, tighten, "").unwrap(),
            Confirm::Done
        );
        let loosen = |l: &mut RiskLimits| l.max_order_quantity = 5_000;
        assert_eq!(
            manager.change_limits(None, loosen, "").unwrap(),
            Confirm::Needs(nivesh_exec::LOOSEN_PHRASE.into())
        );

        // A restart keeps the tightened limit, and the kill switch off.
        let restarted = PaperManager::new(Arc::clone(&store));
        let view = restarted.risk_view(&[]);
        assert_eq!(view.limits.max_order_quantity, 10);
        assert!(!view.kill.engaged);
        assert_eq!(
            restarted
                .change_limits(None, loosen, nivesh_exec::LOOSEN_PHRASE)
                .unwrap(),
            Confirm::Done
        );
        assert_eq!(restarted.risk_view(&[]).limits.max_order_quantity, 5_000);
    }

    #[test]
    fn unreadable_limits_fail_closed() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        store.set_setting(LIMITS_SETTING, "{not json").unwrap();
        let manager = PaperManager::new(store);
        let view = manager.risk_view(&[]);
        assert_eq!(view.limits, RiskLimits::default());
        assert!(view.kill.engaged, "the kill switch holds until reviewed");
    }

    #[test]
    fn every_start_is_disarmed_with_live_trading_off() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let manager = PaperManager::new(Arc::clone(&store));
        assert_eq!(
            manager.set_live_enabled(true, "").unwrap(),
            Confirm::Needs(nivesh_exec::LIVE_PHRASE.into())
        );
        assert_eq!(
            manager
                .set_live_enabled(true, nivesh_exec::LIVE_PHRASE)
                .unwrap(),
            Confirm::Done
        );
        assert!(manager.risk_view(&[]).live_enabled);
        // Without a broker that can trade, nothing arms, phrase or not.
        let id = store
            .save_strategy(None, "Test", "MA Crossover", "{}", 0)
            .unwrap();
        let err = manager
            .arm(id, "Test", &nivesh_exec::arm_phrase("Test"), true)
            .unwrap_err();
        assert!(err.message.contains("not running"), "{}", err.message);

        let restarted = PaperManager::new(store);
        let view = restarted.risk_view(&[id]);
        assert!(!view.live_enabled, "the master switch is never saved");
        assert_eq!(view.strategies[0].armed_until, None);
    }

    #[test]
    fn an_unreadable_kill_switch_fails_closed() {
        let store = Store::open_in_memory().unwrap();
        store.set_setting(KILL_SETTING, "not json").unwrap();
        assert!(load_kill(&store).engaged);
    }

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
        lock(&desk).start(
            id,
            PaperRunner::new(set, "NSE:TCS", PAPER_EQUITY).unwrap(),
            now_ms(),
        );
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
        assert!(
            on_closed_bar(
                id,
                bar(t0, 100.0),
                Freshness::Delayed,
                &desk,
                &store,
                &mut record,
                &Notifier::default(),
            )
            .is_some()
        );
        assert!(
            on_closed_bar(
                id,
                bar(t0 + 60_000, 101.0),
                Freshness::Delayed,
                &desk,
                &store,
                &mut record,
                &Notifier::default(),
            )
            .is_some()
        );

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

        lock(&desk).stop(id, now_ms());
        assert!(
            on_closed_bar(
                id,
                bar(t0 + 120_000, 102.0),
                Freshness::Delayed,
                &desk,
                &store,
                &mut record,
                &Notifier::default(),
            )
            .is_none()
        );
    }
}
