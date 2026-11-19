//! Reconciliation: Nivesh's view of its live orders and positions, checked
//! against the broker's own books (PLAN.md §11.7).
//!
//! [`LiveBook`] is what Nivesh believes: every order it sent (by client
//! tag, with what it last knew) and the shares each strategy holds.
//! [`reconcile`] compares it with the broker's order book, positions and
//! holdings and lists every [`Mismatch`]. The rules fail closed:
//!
//! - an order whose placement was ambiguous is resolved by its tag: found
//!   means it exists (adopt the broker's view); not found stays *unknown*;
//! - an order the broker doesn't have, a different status or fill, or a
//!   Nivesh-tagged order Nivesh doesn't know, are mismatches;
//! - holding **fewer** shares than Nivesh believes is a mismatch (it could
//!   not sell what it thinks it owns). Holding more is fine: the user may
//!   own the same stock themselves.
//!
//! Each mismatch names the strategy it affects. The harness pauses those
//! strategies (no new intents) and raises a `risk:alert`. Reconciliation
//! runs at startup, on every reconnect and periodically; the harness
//! schedules it, this module only decides.

use std::collections::{BTreeMap, BTreeSet};

use nivesh_broker::{BrokerAccount, BrokerError, BrokerOrderId, Order, OrderStatus, OrderTag};
use nivesh_core::{InstrumentId, Side};
use serde::Serialize;

/// What Nivesh knows about one order it sent.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackedOrder {
    pub tag: OrderTag,
    pub strategy_id: i64,
    pub id: InstrumentId,
    pub side: Side,
    pub quantity: i64,
    /// The broker's id, once an acknowledgement arrived. `None` after an
    /// ambiguous placement.
    pub broker_id: Option<BrokerOrderId>,
    /// The last status Nivesh saw; `None` when the placement was ambiguous.
    pub status: Option<OrderStatus>,
    pub filled_quantity: i64,
}

/// Nivesh's live book.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LiveBook {
    pub orders: Vec<TrackedOrder>,
    /// Shares each strategy holds, per instrument.
    pub holdings: BTreeMap<(i64, InstrumentId), i64>,
}

impl LiveBook {
    /// Shares Nivesh believes it holds in `id`, across strategies.
    fn held(&self, id: &InstrumentId) -> i64 {
        self.holdings
            .iter()
            .filter(|((_, i), _)| i == id)
            .map(|(_, q)| *q)
            .sum()
    }
}

/// One disagreement, and the strategy (or strategies) it affects.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Mismatch {
    /// An ambiguous placement the broker has no order for. Its fate stays
    /// unknown: it is not placed again.
    UnknownOutcome { strategy_id: i64, tag: String },
    /// Nivesh believes the order is live; the broker has no such order.
    MissingAtBroker { strategy_id: i64, tag: String },
    /// The broker reports a different status.
    StatusDiffers {
        strategy_id: i64,
        tag: String,
        ours: String,
        broker: String,
    },
    /// The broker reports a different filled quantity.
    FillDiffers {
        strategy_id: i64,
        tag: String,
        ours: i64,
        broker: i64,
    },
    /// A Nivesh-tagged order at the broker that Nivesh does not track
    /// (another session, another device).
    UntrackedOrder { tag: String, broker_id: String },
    /// The broker holds fewer shares than Nivesh's strategies believe.
    PositionShort {
        strategy_ids: Vec<i64>,
        symbol: String,
        ours: i64,
        broker: i64,
    },
}

impl Mismatch {
    /// The strategies to pause. An untracked order affects no known
    /// strategy but still raises the alert.
    pub fn strategies(&self) -> Vec<i64> {
        match self {
            Self::UnknownOutcome { strategy_id, .. }
            | Self::MissingAtBroker { strategy_id, .. }
            | Self::StatusDiffers { strategy_id, .. }
            | Self::FillDiffers { strategy_id, .. } => vec![*strategy_id],
            Self::PositionShort { strategy_ids, .. } => strategy_ids.clone(),
            Self::UntrackedOrder { .. } => Vec::new(),
        }
    }
}

/// What a reconciliation found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reconciliation {
    pub mismatches: Vec<Mismatch>,
    /// Ambiguous placements the broker turned out to have: the book
    /// should adopt these (tag → the broker's order).
    pub resolved: Vec<(OrderTag, Order)>,
}

impl Reconciliation {
    pub fn is_clean(&self) -> bool {
        self.mismatches.is_empty()
    }

    /// Every strategy that must pause.
    pub fn strategies_to_pause(&self) -> BTreeSet<i64> {
        self.mismatches
            .iter()
            .flat_map(Mismatch::strategies)
            .collect()
    }
}

fn label(status: OrderStatus) -> String {
    format!("{status:?}")
}

/// Compares the book with the broker's state. Pure.
pub fn reconcile(
    book: &LiveBook,
    broker_orders: &[Order],
    broker_shares: &BTreeMap<InstrumentId, i64>,
) -> Reconciliation {
    let mut out = Reconciliation::default();
    let by_tag = |tag: &OrderTag| {
        broker_orders
            .iter()
            .find(|o| o.tag.as_deref() == Some(tag.as_str()))
    };
    for tracked in &book.orders {
        let tag = tracked.tag.as_str().to_owned();
        let strategy_id = tracked.strategy_id;
        match (tracked.status, by_tag(&tracked.tag)) {
            (None, Some(order)) => out.resolved.push((tracked.tag.clone(), order.clone())),
            (None, None) => out
                .mismatches
                .push(Mismatch::UnknownOutcome { strategy_id, tag }),
            (Some(ours), None) => {
                // An order Nivesh saw rejected or cancelled may age out of
                // the broker's day book; anything else must be there.
                if !matches!(ours, OrderStatus::Rejected | OrderStatus::Cancelled) {
                    out.mismatches
                        .push(Mismatch::MissingAtBroker { strategy_id, tag });
                }
            }
            (Some(ours), Some(order)) => {
                if ours != order.status {
                    out.mismatches.push(Mismatch::StatusDiffers {
                        strategy_id,
                        tag: tag.clone(),
                        ours: label(ours),
                        broker: label(order.status),
                    });
                }
                if tracked.filled_quantity != order.filled_quantity {
                    out.mismatches.push(Mismatch::FillDiffers {
                        strategy_id,
                        tag,
                        ours: tracked.filled_quantity,
                        broker: order.filled_quantity,
                    });
                }
            }
        }
    }
    for order in broker_orders {
        if let Some(tag) = order.tag.as_deref()
            && OrderTag::is_nivesh(tag)
            && !book.orders.iter().any(|t| t.tag.as_str() == tag)
        {
            out.mismatches.push(Mismatch::UntrackedOrder {
                tag: tag.to_owned(),
                broker_id: order.id.0.clone(),
            });
        }
    }
    let instruments: BTreeSet<&InstrumentId> = book.holdings.keys().map(|(_, id)| id).collect();
    for id in instruments {
        let ours = book.held(id);
        let broker = broker_shares.get(id).copied().unwrap_or(0);
        if broker < ours {
            out.mismatches.push(Mismatch::PositionShort {
                strategy_ids: book
                    .holdings
                    .iter()
                    .filter(|((_, i), q)| i == id && **q > 0)
                    .map(|((s, _), _)| *s)
                    .collect(),
                symbol: id.to_string(),
                ours,
                broker,
            });
        }
    }
    out
}

/// The shares the account holds per instrument: today's net positions plus
/// settled holdings (delivery buys move from one to the other overnight).
pub fn broker_shares(
    positions: &nivesh_broker::Positions,
    holdings: &[nivesh_broker::Holding],
) -> BTreeMap<InstrumentId, i64> {
    let mut shares = BTreeMap::new();
    for h in holdings {
        *shares.entry(h.id.clone()).or_insert(0) += h.quantity;
    }
    for p in &positions.net {
        *shares.entry(p.id.clone()).or_insert(0) += p.net_quantity;
    }
    shares
}

/// Reads the broker's books and reconciles. A broker that can't be read is
/// an error: the caller treats that as a failure to reconcile (fail closed).
pub async fn reconcile_with(
    broker: &dyn BrokerAccount,
    book: &LiveBook,
) -> Result<Reconciliation, BrokerError> {
    let orders = broker.orders().await?;
    let positions = broker.positions().await?;
    let holdings = broker.holdings().await?;
    Ok(reconcile(
        book,
        &orders,
        &broker_shares(&positions, &holdings),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use nivesh_broker::{
        BrokerId, Funds, Holding, OrderKind, Position, Positions, Product, SessionStatus, Trade,
    };
    use nivesh_core::Exchange;

    fn id(s: &str) -> InstrumentId {
        InstrumentId::new(Exchange::Nse, s).unwrap()
    }

    fn tracked(seq: u64, status: Option<OrderStatus>, filled: i64) -> TrackedOrder {
        TrackedOrder {
            tag: OrderTag::new(1, seq).unwrap(),
            strategy_id: 1,
            id: id("RELIANCE"),
            side: Side::Buy,
            quantity: 10,
            broker_id: status.map(|_| BrokerOrderId(format!("B{seq}"))),
            status,
            filled_quantity: filled,
        }
    }

    fn broker_order(tag: &str, status: OrderStatus, filled: i64) -> Order {
        Order {
            id: BrokerOrderId(format!("B-{tag}")),
            id_instrument: id("RELIANCE"),
            side: Side::Buy,
            kind: OrderKind::Limit,
            product: Product::Cnc,
            quantity: 10,
            filled_quantity: filled,
            price: Some(100_000),
            average_price: None,
            status,
            placed_at: None,
            tag: Some(tag.to_owned()),
        }
    }

    fn book(orders: Vec<TrackedOrder>, held: i64) -> LiveBook {
        let mut holdings = BTreeMap::new();
        if held > 0 {
            holdings.insert((1, id("RELIANCE")), held);
        }
        LiveBook { orders, holdings }
    }

    fn shares(n: i64) -> BTreeMap<InstrumentId, i64> {
        BTreeMap::from([(id("RELIANCE"), n)])
    }

    #[test]
    fn agreeing_books_are_clean() {
        let b = book(vec![tracked(1, Some(OrderStatus::Filled), 10)], 10);
        let r = reconcile(
            &b,
            &[broker_order("NV1S1", OrderStatus::Filled, 10)],
            &shares(10),
        );
        assert!(r.is_clean(), "{:?}", r.mismatches);
        // The user owning more of the stock themselves is fine.
        assert!(
            reconcile(
                &b,
                &[broker_order("NV1S1", OrderStatus::Filled, 10)],
                &shares(50)
            )
            .is_clean()
        );
    }

    #[test]
    fn an_ambiguous_placement_is_resolved_by_tag_or_stays_unknown() {
        let b = book(vec![tracked(1, None, 0)], 0);
        let found = reconcile(
            &b,
            &[broker_order("NV1S1", OrderStatus::Open, 0)],
            &shares(0),
        );
        assert!(found.is_clean());
        assert_eq!(found.resolved.len(), 1);
        assert_eq!(found.resolved[0].1.status, OrderStatus::Open);

        let lost = reconcile(&b, &[], &shares(0));
        assert_eq!(
            lost.mismatches,
            vec![Mismatch::UnknownOutcome {
                strategy_id: 1,
                tag: "NV1S1".into()
            }]
        );
        assert_eq!(lost.strategies_to_pause(), BTreeSet::from([1]));
    }

    #[test]
    fn each_kind_of_mismatch_is_found() {
        // Open at ours, missing at the broker.
        let missing = reconcile(
            &book(vec![tracked(1, Some(OrderStatus::Open), 0)], 0),
            &[],
            &shares(0),
        );
        assert!(matches!(
            &missing.mismatches[..],
            [Mismatch::MissingAtBroker { .. }]
        ));
        // A rejected one aging out of the day book is fine.
        assert!(
            reconcile(
                &book(vec![tracked(1, Some(OrderStatus::Rejected), 0)], 0),
                &[],
                &shares(0)
            )
            .is_clean()
        );

        // Status and fill disagree.
        let b = book(vec![tracked(1, Some(OrderStatus::Open), 0)], 0);
        let r = reconcile(
            &b,
            &[broker_order("NV1S1", OrderStatus::PartiallyFilled, 4)],
            &shares(4),
        );
        assert!(matches!(
            &r.mismatches[..],
            [Mismatch::StatusDiffers { ours, broker, .. }, Mismatch::FillDiffers { ours: 0, broker: 4, .. }]
                if ours == "Open" && broker == "PartiallyFilled"
        ));

        // A Nivesh-tagged order nobody tracks; the user's own orders are
        // not Nivesh's business.
        let mut stranger = broker_order("NV9S3", OrderStatus::Open, 0);
        stranger.id = BrokerOrderId("X1".into());
        let mut users = broker_order("manual", OrderStatus::Open, 0);
        users.tag = Some("manual".into());
        let r = reconcile(&book(vec![], 0), &[stranger, users], &shares(0));
        assert_eq!(
            r.mismatches,
            vec![Mismatch::UntrackedOrder {
                tag: "NV9S3".into(),
                broker_id: "X1".into()
            }]
        );
        assert!(
            r.strategies_to_pause().is_empty(),
            "no known strategy to pause"
        );

        // Fewer shares at the broker than the strategies believe.
        let r = reconcile(&book(vec![], 10), &[], &shares(4));
        assert_eq!(
            r.mismatches,
            vec![Mismatch::PositionShort {
                strategy_ids: vec![1],
                symbol: "NSE:RELIANCE".into(),
                ours: 10,
                broker: 4
            }]
        );
    }

    #[test]
    fn shares_count_positions_and_holdings_together() {
        let positions = Positions {
            day: vec![],
            net: vec![Position {
                id: id("RELIANCE"),
                net_quantity: 3,
                day_quantity: 3,
                average_price: 0,
                last_price: None,
                pnl: None,
                realised: None,
            }],
        };
        let holdings = vec![Holding {
            id: id("RELIANCE"),
            quantity: 7,
            average_price: 0,
            last_price: None,
            pnl: None,
        }];
        assert_eq!(broker_shares(&positions, &holdings), shares(10));
    }

    /// A broker that answers from fixed books, or fails.
    struct MockBroker {
        orders: Vec<Order>,
        held: i64,
        down: bool,
    }

    #[async_trait]
    impl BrokerAccount for MockBroker {
        fn broker_id(&self) -> BrokerId {
            BrokerId::Demo
        }
        async fn session(&self) -> Result<SessionStatus, BrokerError> {
            Err(BrokerError::Unauthorized)
        }
        async fn holdings(&self) -> Result<Vec<Holding>, BrokerError> {
            Ok(vec![Holding {
                id: id("RELIANCE"),
                quantity: self.held,
                average_price: 0,
                last_price: None,
                pnl: None,
            }])
        }
        async fn positions(&self) -> Result<Positions, BrokerError> {
            Ok(Positions::default())
        }
        async fn orders(&self) -> Result<Vec<Order>, BrokerError> {
            if self.down {
                return Err(BrokerError::Timeout);
            }
            Ok(self.orders.clone())
        }
        async fn trades(&self) -> Result<Vec<Trade>, BrokerError> {
            Ok(vec![])
        }
        async fn funds(&self) -> Result<Funds, BrokerError> {
            Err(BrokerError::Unauthorized)
        }
    }

    #[tokio::test]
    async fn reconciling_against_a_broker_reads_its_books() {
        let b = book(vec![tracked(1, Some(OrderStatus::Filled), 10)], 10);
        let ok = MockBroker {
            orders: vec![broker_order("NV1S1", OrderStatus::Filled, 10)],
            held: 10,
            down: false,
        };
        assert!(reconcile_with(&ok, &b).await.unwrap().is_clean());
        let short = MockBroker { held: 0, ..ok };
        assert_eq!(
            reconcile_with(&short, &b)
                .await
                .unwrap()
                .strategies_to_pause(),
            BTreeSet::from([1])
        );
        let down = MockBroker {
            orders: vec![],
            held: 0,
            down: true,
        };
        assert_eq!(reconcile_with(&down, &b).await, Err(BrokerError::Timeout));
    }
}
