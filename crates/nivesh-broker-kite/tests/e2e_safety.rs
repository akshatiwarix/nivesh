//! End-to-end safety scenarios (PLAN.md §10 Part 6 item 13): the real
//! desk, gateway, risk engine, hash-chained audit log and Kite adapter,
//! against a mock Kite server. No real broker, no real credentials.
//!
//! The Kite adapter checks approvals against the wall clock, so the desk
//! runs on a clock inside the next NSE session that is never behind the
//! real one.

#[cfg(test)]
#[cfg(feature = "live-trading")]
mod e2e {
    use std::sync::Arc;

    use nivesh_broker::{BrokerAccount as _, OrderTag};
    use nivesh_broker_kite::{ExecutionConfig, KiteAccount, KiteExecution, KiteRest};
    use nivesh_core::{Candle, Freshness};
    use nivesh_exec::{
        ArmConditions, AuditEvent, AuditSink, GatewayError, KillState, LIVE_PHRASE, LiveRoute,
        NextOpen, PaperDesk, arm_phrase, reconcile_with,
    };
    use nivesh_risk::{RiskCheck, RiskEngine, RiskLimits};
    use nivesh_script::{Script, ScriptLimits};
    use nivesh_secrets::{InMemorySecretStore, SecretStore as _};
    use nivesh_store::{AuditEntry, AuditQuery, Store};
    use nivesh_strategy::{PaperRunner, Rule, RuleSet, Sizing};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const MIN: i64 = 60_000;
    const DAY: i64 = 86_400_000;
    const IST: i64 = 19_800_000;
    const NAME: &str = "E2E";
    const SYMBOL: &str = "NSE:RELIANCE";

    /// 10:30 IST today, or tomorrow if that has passed.
    fn session_now() -> i64 {
        let real = nivesh_risk::now_ms();
        let ist = real + IST;
        let at = ist - ist.rem_euclid(DAY) - IST + (10 * 60 + 30) * MIN;
        if at < real { at + DAY } else { at }
    }

    /// The app's audit sink: the store's hash chain.
    struct StoreAudit(Arc<Store>);

    impl AuditSink for StoreAudit {
        fn record(&self, at_ms: i64, event: &AuditEvent) -> Result<(), String> {
            let payload = event.payload();
            self.0
                .append_audit(&AuditEntry {
                    at_ms,
                    kind: event.kind(),
                    strategy_id: event.strategy_id(),
                    payload: &payload,
                })
                .map(drop)
                .map_err(|e| e.to_string())
        }
    }

    fn kinds(store: &Store) -> Vec<String> {
        store
            .audit(&AuditQuery::default())
            .unwrap()
            .into_iter()
            .rev()
            .map(|r| r.kind)
            .collect()
    }

    fn bar(now: i64, i: i64, price: f64) -> Candle {
        Candle {
            t: now + i * MIN,
            o: price,
            h: price + 1.0,
            l: price - 1.0,
            c: price,
            v: 1_000.0,
        }
    }

    fn buys(qty: i64) -> RuleSet {
        RuleSet {
            entry: Rule::True,
            exit: Rule::False,
            sizing: Sizing::FixedQuantity(qty),
            ..RuleSet::default()
        }
    }

    fn ok(order_id: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_string(format!(
            r#"{{"status":"success","data":{{"order_id":"{order_id}"}}}}"#
        ))
    }

    fn data(json: &str) -> ResponseTemplate {
        ResponseTemplate::new(200)
            .set_body_string(format!(r#"{{"status":"success","data":{json}}}"#))
    }

    fn open_order(id: &str, tag: &str) -> String {
        format!(
            r#"{{"order_id":"{id}","exchange":"NSE","tradingsymbol":"RELIANCE",
                "transaction_type":"BUY","order_type":"LIMIT","product":"CNC",
                "quantity":1,"filled_quantity":0,"price":100.0,"average_price":0,
                "status":"OPEN","order_timestamp":"2026-11-20 10:15:00","tag":"{tag}"}}"#
        )
    }

    fn kite(server: &MockServer) -> Arc<KiteExecution> {
        let rest = Arc::new(KiteRest::new(Some(server.uri()), "testkey".into()).unwrap());
        rest.set_token(secrecy::SecretString::from("day-token"));
        let secrets = Arc::new(InMemorySecretStore::new());
        secrets
            .put(
                nivesh_broker_kite::KEY_ACCESS_TOKEN,
                secrecy::SecretString::from("day-token"),
            )
            .unwrap();
        Arc::new(KiteExecution::new(
            KiteAccount::new(Arc::clone(&rest), secrets),
            rest,
            ExecutionConfig::default(),
        ))
    }

    struct World {
        store: Arc<Store>,
        desk: PaperDesk,
        kite: Arc<KiteExecution>,
        now: i64,
    }

    /// The desk over a fresh store, routed to the mock Kite, with strategy
    /// 1 running `set`. `armed`: live trading on and strategy 1 armed.
    fn world(server: &MockServer, set: RuleSet, limits: RiskLimits, armed: bool) -> World {
        let now = session_now();
        let store = Arc::new(Store::open_in_memory().unwrap());
        let audit = Arc::new(StoreAudit(Arc::clone(&store)));
        let mut desk = PaperDesk::with_broker(
            RiskEngine::new(limits),
            Arc::new(NextOpen::FREE),
            audit.clone(),
        );
        desk.start(1, PaperRunner::new(set, SYMBOL, 100_000.0).unwrap(), now);
        desk.set_strategy_limits(1, Some(RiskLimits::default()));
        let kite = kite(server);
        desk.install_live_route(Arc::new(LiveRoute::new(kite.clone(), audit)));
        if armed {
            desk.set_live_enabled(true, LIVE_PHRASE, now).unwrap();
            let live = ArmConditions {
                broker_connected: true,
                data: Freshness::Live,
            };
            desk.arm(1, NAME, &arm_phrase(NAME), live, now).unwrap();
        }
        World {
            store,
            desk,
            kite,
            now,
        }
    }

    async fn no_order_placed(server: &MockServer) {
        Mock::given(method("POST"))
            .and(path("/orders/regular"))
            .respond_with(ok("never"))
            .expect(0)
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn an_armed_strategys_order_is_audited_then_placed_once_at_kite() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/orders/regular"))
            .respond_with(ok("9001"))
            .expect(1)
            .mount(&server)
            .await;
        let mut w = world(&server, buys(1), RiskLimits::default(), true);
        let mut r = w
            .desk
            .on_closed_bar(1, bar(w.now, 0, 100.0), Freshness::Live, w.now)
            .unwrap();
        let dispatch = r.dispatches.pop().unwrap();
        assert_eq!(
            kinds(&w.store).last().unwrap(),
            "order.routed",
            "on the record first"
        );
        let ack = dispatch.send(w.now).await.unwrap();
        assert_eq!(ack.id.0, "9001");
        let k = kinds(&w.store);
        assert_eq!(&k[k.len() - 2..], ["order.routed", "order.acked"]);
        assert!(w.store.verify_audit().unwrap().is_intact());
    }

    #[tokio::test]
    async fn a_limit_breach_is_rejected_and_audited_and_never_reaches_kite() {
        let server = MockServer::start().await;
        no_order_placed(&server).await;
        // 2,000 shares: over the 1,000-share cap.
        let mut w = world(&server, buys(2_000), RiskLimits::default(), true);
        let r = w
            .desk
            .on_closed_bar(1, bar(w.now, 0, 100.0), Freshness::Live, w.now)
            .unwrap();
        assert!(matches!(
            r.refused.as_slice(),
            [GatewayError::RiskRejected(rej)] if rej.check == RiskCheck::Quantity
        ));
        assert!(r.dispatches.is_empty());
        let rejected = w
            .store
            .audit(&AuditQuery {
                kind_prefix: Some("risk.rejected".into()),
                ..AuditQuery::default()
            })
            .unwrap();
        assert_eq!(rejected.len(), 1);
        assert!(
            rejected[0].payload.contains("quantity"),
            "{}",
            rejected[0].payload
        );
    }

    #[tokio::test]
    async fn delayed_data_never_trades_live() {
        let server = MockServer::start().await;
        no_order_placed(&server).await;
        let mut w = world(&server, buys(1), RiskLimits::default(), true);
        let r = w
            .desk
            .on_closed_bar(1, bar(w.now, 0, 100.0), Freshness::Delayed, w.now)
            .unwrap();
        assert_eq!(
            r.refused,
            vec![GatewayError::NotLiveData(Freshness::Delayed)]
        );
        assert!(r.dispatches.is_empty());
        assert_eq!(w.desk.queued(), 0, "not to paper either");
        assert!(kinds(&w.store).contains(&"intent.not_live".to_owned()));
    }

    #[tokio::test]
    async fn a_daily_loss_breach_engages_the_kill_switch_which_cancels_and_survives_a_restart() {
        let server = MockServer::start().await;
        // An open Nivesh order at Kite (from an earlier bar), and the user's own.
        Mock::given(method("GET"))
            .and(path("/orders"))
            .respond_with(data(&format!(
                "[{}, {}]",
                open_order("8001", "NV1S99"),
                open_order("8002", "")
            )))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/orders/regular/8001"))
            .respond_with(ok("8001"))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/orders/regular/8002"))
            .respond_with(ok("8002"))
            .expect(0)
            .mount(&server)
            .await;
        // A ₹100 daily-loss limit; a paper strategy buying 10 shares.
        let limits = RiskLimits {
            max_daily_loss_paise: 100 * 100,
            ..RiskLimits::default()
        };
        let mut w = world(&server, buys(10), limits, false);
        w.desk
            .on_closed_bar(1, bar(w.now, 0, 100.0), Freshness::Live, w.now)
            .unwrap();
        w.desk
            .on_closed_bar(1, bar(w.now, 1, 100.0), Freshness::Live, w.now + MIN)
            .unwrap();
        // Down ₹20 a share on 10 shares: a ₹200 loss.
        let r = w
            .desk
            .on_closed_bar(1, bar(w.now, 2, 80.0), Freshness::Live, w.now + 2 * MIN)
            .unwrap();
        let kill = r.kill.expect("the loss trips the switch");
        assert!(kill.engaged);

        // The broker half: what the app runs on every trip.
        let report = w
            .desk
            .live_route()
            .unwrap()
            .cancel_open(w.now + 2 * MIN)
            .await
            .unwrap();
        assert_eq!(report.cancelled, vec!["NV1S99"]);

        // A restart: the saved state comes back engaged and blocks orders.
        w.store
            .set_setting("kill_switch", &serde_json::to_string(&kill).unwrap())
            .unwrap();
        let saved: KillState =
            serde_json::from_str(&w.store.setting("kill_switch").unwrap().unwrap()).unwrap();
        let mut restarted = PaperDesk::with_broker(
            RiskEngine::new(RiskLimits::default()),
            Arc::new(NextOpen::FREE),
            Arc::new(StoreAudit(Arc::clone(&w.store))),
        );
        restarted.restore_kill(saved);
        restarted.start(
            1,
            PaperRunner::new(buys(1), SYMBOL, 100_000.0).unwrap(),
            w.now,
        );
        let r = restarted
            .on_closed_bar(1, bar(w.now, 3, 80.0), Freshness::Live, w.now + 3 * MIN)
            .unwrap();
        assert_eq!(r.refused, vec![GatewayError::KillSwitchEngaged]);
        let k = kinds(&w.store);
        assert!(k.contains(&"kill.engaged".to_owned()));
        assert!(k.contains(&"order.cancel".to_owned()));
        assert!(k.contains(&"kill.blocked".to_owned()));
        assert!(w.store.verify_audit().unwrap().is_intact());
    }

    #[tokio::test]
    async fn an_ambiguous_placement_is_reconciled_by_tag_not_retried() {
        let server = MockServer::start().await;
        // Kite answers 503: the order may or may not exist.
        Mock::given(method("POST"))
            .and(path("/orders/regular"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;
        let mut w = world(&server, buys(1), RiskLimits::default(), true);
        let mut r = w
            .desk
            .on_closed_bar(1, bar(w.now, 0, 100.0), Freshness::Live, w.now)
            .unwrap();
        let dispatch = r.dispatches.pop().unwrap();
        let tag: OrderTag = dispatch.tag().clone();
        let outcome = dispatch.send(w.now).await;
        assert!(matches!(outcome, Err(GatewayError::AmbiguousPlacement(_))));
        w.desk.on_live_outcome(1, &outcome, w.now);
        assert!(w.desk.paused(1).is_some(), "paused until reconciled");

        // The next bar places nothing: the strategy is paused.
        let r = w
            .desk
            .on_closed_bar(1, bar(w.now, 1, 100.0), Freshness::Live, w.now + MIN)
            .unwrap();
        assert!(r.dispatches.is_empty());

        // Kite did take it: reconciliation finds it by its tag.
        Mock::given(method("GET"))
            .and(path("/orders"))
            .respond_with(data(&format!("[{}]", open_order("7777", tag.as_str()))))
            .mount(&server)
            .await;
        for p in ["/portfolio/positions"] {
            Mock::given(method("GET"))
                .and(path(p))
                .respond_with(data(r#"{"net":[],"day":[]}"#))
                .mount(&server)
                .await;
        }
        Mock::given(method("GET"))
            .and(path("/portfolio/holdings"))
            .respond_with(data("[]"))
            .mount(&server)
            .await;
        let book = w.desk.live_route().unwrap().book();
        assert_eq!(book.orders[0].broker_id, None);
        let found = reconcile_with(w.kite.as_ref(), &book).await.unwrap();
        assert_eq!(found.resolved.len(), 1);
        assert_eq!(found.resolved[0].0, tag);
        assert_eq!(found.resolved[0].1.id.0, "7777");
        // The mock's expect(1) on POST checks it was never placed again.
        assert!(w.kite.orders().await.is_ok());
    }

    #[tokio::test]
    async fn a_runaway_script_decides_nothing_and_places_nothing() {
        let server = MockServer::start().await;
        no_order_placed(&server).await;
        let mut w = world(&server, buys(1), RiskLimits::default(), true);
        // Swap strategy 1 for a script that never returns.
        let script = Script::new("fn on_bar(bar) { loop { } }", ScriptLimits::default())
            .unwrap()
            .with_symbol(SYMBOL);
        w.desk.start(
            1,
            PaperRunner::with_decider(Box::new(script), SYMBOL, 100_000.0),
            w.now,
        );
        let started = std::time::Instant::now();
        let r = w
            .desk
            .on_closed_bar(1, bar(w.now, 0, 100.0), Freshness::Live, w.now)
            .unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        assert!(r.placed.is_none() && r.dispatches.is_empty() && r.refused.is_empty());
        // Restarting it also disarmed it: scripts are paper until armed again.
        assert_eq!(w.desk.armed_until(1, w.now), None);
    }
}
