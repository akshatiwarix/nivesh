//! Live arming, end to end through the desk and a mock broker: every start
//! is paper and disarmed, each precondition blocks arming, an armed
//! strategy never trades on DELAYED data, and its live orders are audited,
//! placed once, and fail closed.

#[cfg(test)]
mod arming {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use nivesh_broker::{
        BrokerAccount, BrokerError, BrokerExecution, BrokerId, BrokerOrderId, Funds, Holding,
        Order, OrderAck, OrderTag, Positions, SessionStatus, Trade,
    };
    use nivesh_core::{Candle, Freshness};
    use nivesh_exec::{
        ArmConditions, ArmError, AuditEvent, GatewayError, KillReason, LIVE_PHRASE, LOOSEN_PHRASE,
        LiveRoute, MemoryAudit, NextOpen, PaperDesk, arm_phrase, session_end_ms,
    };
    use nivesh_risk::{ApprovedOrder, RiskEngine, RiskLimits};
    use nivesh_strategy::{PaperRunner, Rule, RuleSet, Sizing};

    /// 10:30 IST.
    const T0: i64 = 1_795_150_800_000;
    const MIN: i64 = 60_000;
    const NAME: &str = "Always in";

    #[derive(Clone, Copy)]
    enum Answer {
        Ack,
        Timeout,
        Refused,
    }

    /// A broker that records what it was asked to place.
    struct MockBroker {
        answer: Answer,
        placed: Mutex<Vec<String>>,
    }

    impl MockBroker {
        fn new(answer: Answer) -> Arc<Self> {
            Arc::new(Self {
                answer,
                placed: Mutex::default(),
            })
        }

        fn placed(&self) -> Vec<String> {
            self.placed.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl BrokerAccount for MockBroker {
        fn broker_id(&self) -> BrokerId {
            BrokerId::Demo
        }
        async fn session(&self) -> Result<SessionStatus, BrokerError> {
            Err(BrokerError::Unsupported)
        }
        async fn holdings(&self) -> Result<Vec<Holding>, BrokerError> {
            Ok(vec![])
        }
        async fn positions(&self) -> Result<Positions, BrokerError> {
            Ok(Positions::default())
        }
        async fn orders(&self) -> Result<Vec<Order>, BrokerError> {
            Ok(vec![])
        }
        async fn trades(&self) -> Result<Vec<Trade>, BrokerError> {
            Ok(vec![])
        }
        async fn funds(&self) -> Result<Funds, BrokerError> {
            Err(BrokerError::Unsupported)
        }
    }

    #[async_trait]
    impl BrokerExecution for MockBroker {
        async fn place(&self, _: ApprovedOrder, tag: &OrderTag) -> Result<OrderAck, BrokerError> {
            self.placed.lock().unwrap().push(tag.as_str().to_owned());
            match self.answer {
                Answer::Ack => Ok(OrderAck {
                    id: BrokerOrderId("B1".into()),
                }),
                Answer::Timeout => Err(BrokerError::Timeout),
                Answer::Refused => Err(BrokerError::InvalidRequest("RMS: margin".into())),
            }
        }
        async fn modify(
            &self,
            _: &BrokerOrderId,
            _: ApprovedOrder,
        ) -> Result<OrderAck, BrokerError> {
            Err(BrokerError::Unsupported)
        }
        async fn cancel(&self, _: &BrokerOrderId) -> Result<OrderAck, BrokerError> {
            Err(BrokerError::Unsupported)
        }
    }

    fn bar(i: i64) -> Candle {
        Candle {
            t: T0 + i * MIN,
            o: 100.0,
            h: 101.0,
            l: 99.0,
            c: 100.0,
            v: 100.0,
        }
    }

    const LIVE: ArmConditions = ArmConditions {
        broker_connected: true,
        data: Freshness::Live,
    };

    struct Harness {
        desk: PaperDesk,
        log: Arc<MemoryAudit>,
        broker: Arc<MockBroker>,
    }

    /// A running, flat strategy 1 that buys 1 share on every bar, with its
    /// own limits; the master switch and the live route as asked.
    fn harness(master: bool, answer: Answer) -> Harness {
        let log = Arc::new(MemoryAudit::default());
        let mut desk = PaperDesk::with_broker(
            RiskEngine::new(RiskLimits::default()),
            Arc::new(NextOpen::FREE),
            log.clone(),
        );
        let set = RuleSet {
            entry: Rule::True,
            exit: Rule::False,
            sizing: Sizing::FixedQuantity(1),
            ..RuleSet::default()
        };
        desk.start(
            1,
            PaperRunner::new(set, "NSE:RELIANCE", 100_000.0).unwrap(),
            T0,
        );
        desk.set_strategy_limits(1, Some(RiskLimits::default()));
        let broker = MockBroker::new(answer);
        desk.install_live_route(Arc::new(LiveRoute::new(broker.clone(), log.clone())));
        if master {
            desk.set_live_enabled(true, LIVE_PHRASE, T0).unwrap();
        }
        Harness { desk, log, broker }
    }

    fn arm(h: &mut Harness, now: i64) -> Result<i64, GatewayError> {
        h.desk.arm(1, NAME, &arm_phrase(NAME), LIVE, now)
    }

    fn refused(result: Result<i64, GatewayError>) -> ArmError {
        match result {
            Err(GatewayError::Arm(e)) => e,
            other => panic!("expected an arming refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_new_desk_is_paper_and_disarmed() {
        let mut h = harness(false, Answer::Ack);
        assert!(!h.desk.live_enabled());
        assert_eq!(h.desk.armed_until(1, T0), None);
        assert_eq!(refused(arm(&mut h, T0)), ArmError::MasterOff);
        // Its first signal goes to paper.
        let r = h
            .desk
            .on_closed_bar(1, bar(0), Freshness::Live, T0)
            .unwrap();
        assert!(r.placed.is_some() && r.dispatches.is_empty());
        assert_eq!(h.desk.queued(), 1, "queued for a paper fill");
    }

    #[test]
    fn arming_with_every_precondition_met_lasts_until_the_close() {
        let mut h = harness(true, Answer::Ack);
        let until = arm(&mut h, T0).unwrap();
        assert_eq!(until, session_end_ms(T0));
        assert_eq!(h.desk.armed_until(1, T0), Some(until));
        assert_eq!(h.log.kinds(), vec!["live.switch", "strategy.armed"]);
    }

    #[test]
    fn each_failed_precondition_blocks_arming() {
        let typed = arm_phrase(NAME);
        let delayed = ArmConditions {
            data: Freshness::Delayed,
            ..LIVE
        };
        let offline = ArmConditions {
            broker_connected: false,
            ..LIVE
        };
        let mut h = harness(true, Answer::Ack);
        assert!(matches!(
            refused(h.desk.arm(1, NAME, "arm always in live", LIVE, T0)),
            ArmError::WrongPhrase { expected } if expected == typed
        ));
        assert_eq!(
            refused(h.desk.arm(1, NAME, &typed, delayed, T0)),
            ArmError::DataNotLive(Freshness::Delayed)
        );
        assert_eq!(
            refused(h.desk.arm(1, NAME, &typed, offline, T0)),
            ArmError::BrokerDisconnected
        );
        assert_eq!(
            refused(h.desk.arm(2, NAME, &typed, LIVE, T0)),
            ArmError::NotRunning
        );
        let close = session_end_ms(T0);
        assert_eq!(refused(arm(&mut h, close)), ArmError::SessionOver);

        h.desk.set_strategy_limits(1, None);
        assert_eq!(refused(arm(&mut h, T0)), ArmError::NoLimits);
        h.desk.set_strategy_limits(1, Some(RiskLimits::default()));

        // A paper order waiting to fill: not flat.
        h.desk
            .on_closed_bar(1, bar(0), Freshness::Live, T0)
            .unwrap();
        assert_eq!(refused(arm(&mut h, T0)), ArmError::NotFlat);
        // Filled: it holds a paper position, still not flat.
        h.desk
            .on_closed_bar(1, bar(1), Freshness::Live, T0 + MIN)
            .unwrap();
        assert_eq!(refused(arm(&mut h, T0 + MIN)), ArmError::NotFlat);

        let mut h = harness(true, Answer::Ack);
        h.desk.remove_live_route(T0);
        assert_eq!(refused(arm(&mut h, T0)), ArmError::NoLiveRoute);

        let mut h = harness(true, Answer::Ack);
        h.desk.engage_kill(KillReason::Manual, "test", T0);
        assert_eq!(refused(arm(&mut h, T0)), ArmError::KillEngaged);

        let mut h = harness(true, Answer::Ack);
        let mismatch = nivesh_exec::Reconciliation {
            mismatches: vec![nivesh_exec::Mismatch::UnknownOutcome {
                strategy_id: 1,
                tag: "NV1S1".into(),
            }],
            resolved: vec![],
        };
        h.desk.apply_reconciliation(&mismatch, T0);
        assert!(matches!(refused(arm(&mut h, T0)), ArmError::Paused(_)));

        // Nothing above armed anything or left a trace in the log.
        assert!(!h.log.kinds().contains(&"strategy.armed"));
    }

    #[test]
    fn the_master_switch_needs_the_audit_log() {
        let log = Arc::new(MemoryAudit::failing());
        let mut desk = PaperDesk::new(RiskEngine::new(RiskLimits::default()), log);
        assert!(matches!(
            desk.set_live_enabled(true, LIVE_PHRASE, T0),
            Err(GatewayError::AuditFailed(_))
        ));
        assert!(!desk.live_enabled(), "fail closed");
    }

    #[test]
    fn an_armed_strategy_refuses_delayed_data() {
        let mut h = harness(true, Answer::Ack);
        arm(&mut h, T0).unwrap();
        let stale = [Freshness::Delayed, Freshness::Eod, Freshness::Stale];
        for (i, freshness) in (0..).zip(stale) {
            let r = h
                .desk
                .on_closed_bar(1, bar(i), freshness, T0 + i * MIN)
                .unwrap();
            assert_eq!(r.refused, vec![GatewayError::NotLiveData(freshness)]);
            assert!(r.placed.is_none() && r.dispatches.is_empty());
            assert!(r.state.position.is_none());
        }
        assert_eq!(h.desk.queued(), 0, "not routed to paper either");
        assert!(h.broker.placed().is_empty());
        assert!(h.log.kinds().contains(&"intent.not_live"));
    }

    #[tokio::test]
    async fn an_armed_strategy_on_live_data_is_audited_then_placed_once() {
        let mut h = harness(true, Answer::Ack);
        arm(&mut h, T0).unwrap();
        let mut r = h
            .desk
            .on_closed_bar(1, bar(0), Freshness::Live, T0)
            .unwrap();
        assert_eq!(h.desk.queued(), 0, "nothing for paper");
        let dispatch = r.dispatches.pop().unwrap();
        assert!(r.dispatches.is_empty());
        let routed = h.log.events();
        assert!(matches!(
            &routed.last().unwrap().1,
            AuditEvent::OrderRouted { route: "live", .. }
        ));
        assert!(
            h.broker.placed().is_empty(),
            "on the record before it is sent"
        );

        let tag = dispatch.tag().as_str().to_owned();
        assert!(OrderTag::is_nivesh(&tag));
        let outcome = dispatch.send(T0).await;
        assert!(outcome.is_ok());
        assert_eq!(h.desk.on_live_outcome(1, &outcome, T0), None);
        assert_eq!(h.broker.placed(), vec![tag.clone()]);
        assert_eq!(h.log.kinds().last(), Some(&"order.acked"));
        let route = Arc::clone(h.desk_route());
        let book = route.book();
        assert_eq!(book.orders.len(), 1);
        assert_eq!(book.orders[0].tag.as_str(), tag);
        assert!(book.orders[0].broker_id.is_some());
    }

    impl Harness {
        fn desk_route(&self) -> &Arc<LiveRoute> {
            self.desk.live_route().unwrap()
        }
    }

    #[tokio::test]
    async fn an_ambiguous_placement_is_tracked_and_pauses_the_strategy() {
        let mut h = harness(true, Answer::Timeout);
        arm(&mut h, T0).unwrap();
        let mut r = h
            .desk
            .on_closed_bar(1, bar(0), Freshness::Live, T0)
            .unwrap();
        let outcome = r.dispatches.pop().unwrap().send(T0).await;
        assert!(matches!(outcome, Err(GatewayError::AmbiguousPlacement(_))));
        h.desk.on_live_outcome(1, &outcome, T0);
        assert!(h.desk.paused(1).is_some());
        assert_eq!(h.desk.armed_until(1, T0), None, "pausing disarms");
        let book = h.desk_route().book();
        assert_eq!(book.orders.len(), 1);
        assert_eq!(book.orders[0].broker_id, None, "reconciled by tag later");
        assert_eq!(h.broker.placed().len(), 1, "never placed again");
        assert!(h.log.kinds().contains(&"order.ambiguous"));
    }

    #[tokio::test]
    async fn a_refused_live_order_returns_the_runner_to_flat() {
        let mut h = harness(true, Answer::Refused);
        arm(&mut h, T0).unwrap();
        let mut r = h
            .desk
            .on_closed_bar(1, bar(0), Freshness::Live, T0)
            .unwrap();
        let outcome = r.dispatches.pop().unwrap().send(T0).await;
        assert!(matches!(outcome, Err(GatewayError::Broker(_))));
        h.desk.on_live_outcome(1, &outcome, T0);
        assert!(h.desk.paused(1).is_none());
        assert!(h.desk.state(1).unwrap().position.is_none());
        assert!(h.desk_route().book().orders.is_empty());
        assert!(h.log.kinds().contains(&"order.failed"));
    }

    #[tokio::test]
    async fn an_expired_approval_is_never_sent() {
        let mut h = harness(true, Answer::Ack);
        arm(&mut h, T0).unwrap();
        let mut r = h
            .desk
            .on_closed_bar(1, bar(0), Freshness::Live, T0)
            .unwrap();
        let late = T0 + 6 * MIN;
        let outcome = r.dispatches.pop().unwrap().send(late).await;
        assert_eq!(outcome, Err(GatewayError::ApprovalExpired));
        assert!(h.broker.placed().is_empty());
    }

    #[test]
    fn every_safety_action_disarms() {
        let disarmed = |h: &Harness| h.desk.armed_until(1, T0).is_none();

        let mut h = harness(true, Answer::Ack);
        arm(&mut h, T0).unwrap();
        h.desk.engage_kill(KillReason::Manual, "test", T0);
        assert!(disarmed(&h), "the kill switch");

        let mut h = harness(true, Answer::Ack);
        arm(&mut h, T0).unwrap();
        h.desk.set_live_enabled(false, "", T0).unwrap();
        assert!(disarmed(&h), "the master switch");

        let mut h = harness(true, Answer::Ack);
        arm(&mut h, T0).unwrap();
        h.desk.remove_live_route(T0);
        assert!(disarmed(&h), "the broker disconnecting");

        let mut h = harness(true, Answer::Ack);
        arm(&mut h, T0).unwrap();
        assert!(h.desk.disarm(1, T0));
        assert!(disarmed(&h), "the user");

        let mut h = harness(true, Answer::Ack);
        arm(&mut h, T0).unwrap();
        let runner = h.desk.stop(1, T0);
        assert!(runner.is_some());
        assert!(disarmed(&h), "stopping");
        assert!(h.log.kinds().contains(&"strategy.disarmed"));
    }

    #[test]
    fn a_restarted_strategy_comes_back_in_paper() {
        let mut h = harness(true, Answer::Ack);
        arm(&mut h, T0).unwrap();
        let set = RuleSet {
            entry: Rule::True,
            exit: Rule::False,
            sizing: Sizing::FixedQuantity(1),
            ..RuleSet::default()
        };
        h.desk.start(
            1,
            PaperRunner::new(set, "NSE:RELIANCE", 100_000.0).unwrap(),
            T0,
        );
        assert_eq!(h.desk.armed_until(1, T0), None);
        let r = h
            .desk
            .on_closed_bar(1, bar(0), Freshness::Live, T0)
            .unwrap();
        assert!(r.dispatches.is_empty());
        assert_eq!(h.desk.queued(), 1, "paper");
    }

    #[test]
    fn after_the_close_an_armed_strategy_falls_back_to_paper() {
        let mut h = harness(true, Answer::Ack);
        let until = arm(&mut h, T0).unwrap();
        let after = until + MIN;
        let late_bar = Candle { t: after, ..bar(0) };
        let r = h
            .desk
            .on_closed_bar(1, late_bar, Freshness::Live, after)
            .unwrap();
        assert!(r.dispatches.is_empty(), "never live past the close");
        assert!(h.log.events().iter().any(|(_, e)| matches!(
            e,
            AuditEvent::StrategyDisarmed { reason, .. } if reason.contains("session ended")
        )));
    }

    fn needs(phrase: &str, result: Result<impl std::fmt::Debug, GatewayError>) {
        match result {
            Err(GatewayError::NeedsConfirmation { phrase: p }) => assert_eq!(p, phrase),
            other => panic!("expected a confirmation challenge, got {other:?}"),
        }
    }

    #[test]
    fn the_master_switch_needs_its_phrase() {
        let mut h = harness(false, Answer::Ack);
        needs(LIVE_PHRASE, h.desk.set_live_enabled(true, "yes", T0));
        assert!(!h.desk.live_enabled());
        h.desk.set_live_enabled(true, LIVE_PHRASE, T0).unwrap();
        assert!(h.desk.live_enabled());
        // Off needs nothing.
        h.desk.set_live_enabled(false, "", T0).unwrap();
        assert!(!h.desk.live_enabled());
    }

    #[test]
    fn tightening_applies_at_once_and_loosening_needs_the_phrase() {
        let mut h = harness(false, Answer::Ack);
        let base = h.desk.risk_limits().clone();
        let tighter = RiskLimits {
            max_order_quantity: 10,
            ..base.clone()
        };
        assert_eq!(
            h.desk.change_risk_limits(tighter.clone(), "", T0),
            Ok(false)
        );
        assert_eq!(h.desk.risk_limits(), &tighter);

        needs(
            LOOSEN_PHRASE,
            h.desk.change_risk_limits(base.clone(), "", T0),
        );
        assert_eq!(h.desk.risk_limits(), &tighter, "unchanged");
        assert_eq!(
            h.desk.change_risk_limits(base.clone(), LOOSEN_PHRASE, T0),
            Ok(true)
        );
        assert_eq!(h.desk.risk_limits(), &base);

        let changes: Vec<bool> = h
            .log
            .events()
            .into_iter()
            .filter_map(|(_, e)| match e {
                AuditEvent::LimitsChanged {
                    strategy_id: None,
                    loosened,
                    ..
                } => Some(loosened),
                _ => None,
            })
            .collect();
        assert_eq!(changes, vec![false, true], "both on the record");
    }

    #[test]
    fn removing_a_strategys_limits_is_a_loosening_and_disarms_it() {
        let mut h = harness(true, Answer::Ack);
        arm(&mut h, T0).unwrap();
        let tighter = RiskLimits {
            max_orders_per_day: 3,
            ..RiskLimits::default()
        };
        assert_eq!(
            h.desk
                .change_strategy_limits(1, Some(tighter.clone()), "", T0),
            Ok(false)
        );
        assert!(
            h.desk.armed_until(1, T0).is_some(),
            "tightening keeps it armed"
        );
        needs(
            LOOSEN_PHRASE,
            h.desk.change_strategy_limits(1, None, "", T0),
        );
        assert_eq!(h.desk.strategy_limits(1), Some(&tighter));
        assert_eq!(
            h.desk.change_strategy_limits(1, None, LOOSEN_PHRASE, T0),
            Ok(true)
        );
        assert_eq!(h.desk.strategy_limits(1), None);
        assert_eq!(h.desk.armed_until(1, T0), None);
    }

    #[test]
    fn a_loosening_that_cannot_be_audited_does_not_happen() {
        let mut desk = PaperDesk::new(
            RiskEngine::new(RiskLimits::default()),
            Arc::new(MemoryAudit::failing()),
        );
        let tighter = RiskLimits {
            max_order_quantity: 10,
            ..RiskLimits::default()
        };
        // Safer never waits on the log...
        assert_eq!(desk.change_risk_limits(tighter.clone(), "", T0), Ok(false));
        // ...riskier does.
        assert!(matches!(
            desk.change_risk_limits(RiskLimits::default(), LOOSEN_PHRASE, T0),
            Err(GatewayError::AuditFailed(_))
        ));
        assert_eq!(desk.risk_limits(), &tighter);
    }
}
