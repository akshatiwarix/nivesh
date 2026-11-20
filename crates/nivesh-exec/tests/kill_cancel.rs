//! The kill switch's broker half: every open Nivesh-tagged order is
//! cancelled, promptly, and nothing else is touched.

#[cfg(test)]
mod kill_cancel {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use async_trait::async_trait;
    use nivesh_broker::{
        BrokerAccount, BrokerError, BrokerExecution, BrokerId, BrokerOrderId, Funds, Holding,
        Order, OrderAck, OrderKind, OrderStatus, OrderTag, Positions, Product, SessionStatus,
        Trade,
    };
    use nivesh_core::{InstrumentId, Side};
    use nivesh_exec::{LiveRoute, MemoryAudit};
    use nivesh_risk::ApprovedOrder;

    fn order(id: &str, tag: Option<&str>, status: OrderStatus) -> Order {
        Order {
            id: BrokerOrderId(id.into()),
            id_instrument: "NSE:RELIANCE".parse::<InstrumentId>().unwrap(),
            side: Side::Buy,
            kind: OrderKind::Limit,
            product: Product::Cnc,
            quantity: 1,
            filled_quantity: 0,
            price: Some(100_000),
            average_price: None,
            status,
            placed_at: None,
            tag: tag.map(str::to_owned),
        }
    }

    struct Book {
        orders: Vec<Order>,
        /// Order ids whose cancel the broker refuses.
        refuse: Vec<&'static str>,
        cancelled: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl BrokerAccount for Book {
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
            Ok(self.orders.clone())
        }
        async fn trades(&self) -> Result<Vec<Trade>, BrokerError> {
            Ok(vec![])
        }
        async fn funds(&self) -> Result<Funds, BrokerError> {
            Err(BrokerError::Unsupported)
        }
    }

    #[async_trait]
    impl BrokerExecution for Book {
        async fn place(&self, _: ApprovedOrder, _: &OrderTag) -> Result<OrderAck, BrokerError> {
            panic!("the kill switch never places");
        }
        async fn modify(
            &self,
            _: &BrokerOrderId,
            _: ApprovedOrder,
        ) -> Result<OrderAck, BrokerError> {
            panic!("the kill switch never modifies");
        }
        async fn cancel(&self, id: &BrokerOrderId) -> Result<OrderAck, BrokerError> {
            if self.refuse.contains(&id.0.as_str()) {
                return Err(BrokerError::InvalidRequest("already filled".into()));
            }
            self.cancelled.lock().unwrap().push(id.0.clone());
            Ok(OrderAck { id: id.clone() })
        }
    }

    #[tokio::test]
    async fn only_open_nivesh_orders_are_cancelled_within_a_second() {
        let broker = Arc::new(Book {
            orders: vec![
                order("1", Some("NV1S1"), OrderStatus::Open),
                order("2", Some("NV2S7"), OrderStatus::PartiallyFilled),
                order("3", None, OrderStatus::Open), // the user's own
                order("4", Some("MYTAG"), OrderStatus::Open), // not Nivesh's
                order("5", Some("NV1S2"), OrderStatus::Filled), // done already
                order("6", Some("NV3S1"), OrderStatus::Pending),
            ],
            refuse: vec!["6"],
            cancelled: Mutex::default(),
        });
        let log = Arc::new(MemoryAudit::default());
        let route = LiveRoute::new(broker.clone(), log.clone());

        let started = Instant::now();
        let report = route.cancel_open(0).await.unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));

        assert_eq!(*broker.cancelled.lock().unwrap(), vec!["1", "2"]);
        assert_eq!(report.cancelled, vec!["NV1S1", "NV2S7"]);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, "NV3S1");
        assert_eq!(
            log.kinds(),
            vec!["order.cancel", "order.cancel", "order.cancel_failed"]
        );
    }
}
