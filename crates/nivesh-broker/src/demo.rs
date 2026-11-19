//! The demo broker: the whole account experience with **no broker account
//! and no network**. Scrubbed fixture data for the account views and a
//! synthetic random-walk tick stream, clearly labelled `demo` everywhere it
//! shows up.
//!
//! The point is demonstrating Nivesh (`LOGIN DEMO` → `PORT`, `POS`, `ORD`,
//! `FUNDS`) without spending anything. It is never a default: nothing
//! touches the demo broker unless the user asks for it, and nothing it
//! shows is real.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use nivesh_core::InstrumentId;
use secrecy::SecretString;

use crate::status::OrderStatus;
use crate::ticks::{Tick, TickBatch, TickMode, TickSubscription};
use crate::types::{
    BrokerId, BrokerOrderId, Funds, Holding, Order, OrderKind, Position, Positions, Product,
    SessionInfo, SessionState, SessionStatus, Side, Trade,
};
use crate::{BrokerAccount, BrokerError, MarketStream};

/// 09:15 IST today (or the next one), as the demo session's expiry.
fn demo_expiry() -> DateTime<Utc> {
    use chrono::Datelike as _;
    let Some(ist) = chrono::FixedOffset::east_opt(5 * 3600 + 30 * 60) else {
        return Utc::now() + chrono::Duration::hours(24);
    };
    let now = Utc::now();
    let today = now.with_timezone(&ist).date_naive();
    let today_open = ist
        .with_ymd_and_hms(today.year(), today.month(), today.day(), 9, 15, 0)
        .single()
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or(now);
    let tomorrow = today.succ_opt().unwrap_or(today);
    let tomorrow_open = ist
        .with_ymd_and_hms(tomorrow.year(), tomorrow.month(), tomorrow.day(), 9, 15, 0)
        .single()
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or(now + Duration::from_secs(3600));
    if now < today_open {
        today_open
    } else {
        tomorrow_open
    }
}

fn id(symbol: &str) -> InstrumentId {
    // The demo universe is all valid NSE symbols by construction.
    match InstrumentId::new(nivesh_core::Exchange::Nse, symbol) {
        Ok(id) => id,
        Err(_) => InstrumentId::new(nivesh_core::Exchange::Nse, "RELIANCE")
            .unwrap_or_else(|_| unreachable!("RELIANCE is a valid NSE symbol")),
    }
}

/// The demo universe with base prices in paise; the tick stream wanders
/// around these.
const BASES: [(&str, i64); 5] = [
    ("RELIANCE", 137_500),
    ("TCS", 342_000),
    ("HDFCBANK", 162_000),
    ("SBIN", 81_200),
    ("INFY", 152_000),
];

fn base_price_for(i: usize) -> i64 {
    BASES[i % BASES.len()].1
}

/// The demo holdings: what the account "owns".
fn holdings() -> Vec<Holding> {
    vec![
        Holding {
            id: id("RELIANCE"),
            quantity: 10,
            average_price: 125_025,
            last_price: Some(137_500),
            pnl: Some(124_750),
        },
        Holding {
            id: id("TCS"),
            quantity: 5,
            average_price: 345_000,
            last_price: Some(342_000),
            pnl: Some(-15_000),
        },
    ]
}

/// The demo positions: one intraday long, one overnight short.
fn positions() -> Positions {
    let sbin = Position {
        id: id("SBIN"),
        net_quantity: 20,
        day_quantity: 20,
        average_price: 81_050,
        last_price: Some(81_200),
        pnl: Some(3_000),
        realised: Some(0),
    };
    Positions {
        day: vec![sbin.clone()],
        net: vec![
            sbin,
            Position {
                id: id("HDFCBANK"),
                net_quantity: -5,
                day_quantity: 0,
                average_price: 160_000,
                last_price: Some(162_000),
                pnl: Some(-10_000),
                realised: Some(0),
            },
        ],
    }
}

/// The demo order book: one of every interesting state.
fn orders() -> Vec<Order> {
    let placed = |h: u32, m: u32, s: u32| {
        Utc.with_ymd_and_hms(2026, 11, 19, h, m, s)
            .single()
            .unwrap_or_else(Utc::now)
    };
    vec![
        Order {
            id: BrokerOrderId("demo-1".into()),
            id_instrument: id("SBIN"),
            side: Side::Buy,
            kind: OrderKind::Limit,
            product: Product::Cnc,
            quantity: 10,
            filled_quantity: 10,
            price: Some(81_250),
            average_price: Some(81_230),
            status: OrderStatus::Filled,
            placed_at: Some(placed(4, 15, 12)),
            tag: Some("demo".into()),
        },
        Order {
            id: BrokerOrderId("demo-2".into()),
            id_instrument: id("TATASTEEL"),
            side: Side::Sell,
            kind: OrderKind::Limit,
            product: Product::Mis,
            quantity: 5,
            filled_quantity: 2,
            price: Some(14_500),
            average_price: None,
            status: OrderStatus::Open,
            placed_at: Some(placed(4, 32, 30)),
            tag: Some("demo".into()),
        },
        Order {
            id: BrokerOrderId("demo-3".into()),
            id_instrument: id("INFY"),
            side: Side::Buy,
            kind: OrderKind::Market,
            product: Product::Mis,
            quantity: 1,
            filled_quantity: 0,
            price: None,
            average_price: None,
            status: OrderStatus::Rejected,
            placed_at: Some(placed(4, 35, 0)),
            tag: Some("demo".into()),
        },
    ]
}

/// The demo fill for the filled order.
fn trades() -> Vec<Trade> {
    vec![Trade {
        order_id: BrokerOrderId("demo-1".into()),
        id_instrument: id("SBIN"),
        side: Side::Buy,
        quantity: 10,
        price: 81_230,
        traded_at: Utc
            .with_ymd_and_hms(2026, 11, 19, 4, 15, 15)
            .single()
            .unwrap_or_else(Utc::now),
    }]
}

/// The demo funds.
fn funds() -> Funds {
    Funds {
        available: 15_234_560,
        used: 1_220_000,
        total: 15_234_560,
    }
}

/// The demo broker's account. Every view shows it under the `demo` name.
pub struct DemoBroker;

#[async_trait]
impl BrokerAccount for DemoBroker {
    fn broker_id(&self) -> BrokerId {
        BrokerId::Demo
    }

    async fn session(&self) -> Result<SessionStatus, BrokerError> {
        Ok(SessionStatus {
            state: SessionState::Connected,
            expires_at: Some(demo_expiry()),
        })
    }

    async fn holdings(&self) -> Result<Vec<Holding>, BrokerError> {
        Ok(holdings())
    }

    async fn positions(&self) -> Result<Positions, BrokerError> {
        Ok(positions())
    }

    async fn orders(&self) -> Result<Vec<Order>, BrokerError> {
        Ok(orders())
    }

    async fn trades(&self) -> Result<Vec<Trade>, BrokerError> {
        Ok(trades())
    }

    async fn funds(&self) -> Result<Funds, BrokerError> {
        Ok(funds())
    }
}

/// The demo tick stream: prices walk around their base every second until
/// the receiver stops listening. Synthetic by design; nothing is live.
pub struct DemoStream;

#[async_trait]
impl MarketStream for DemoStream {
    async fn subscribe(
        &self,
        ids: &[InstrumentId],
        mode: TickMode,
    ) -> Result<TickSubscription, BrokerError> {
        let (sender, receiver) = tokio::sync::mpsc::channel(32);
        let ids = ids.to_vec();
        tokio::spawn(async move {
            let mut step = 0i64;
            loop {
                let ticks: Vec<Tick> = ids
                    .iter()
                    .enumerate()
                    .map(|(i, vid)| {
                        let base = base_price_for(i);
                        // A deterministic walk that wanders ±10 ticks.
                        let wobble = ((step * (i as i64 + 3)) % 21) - 10;
                        Tick {
                            id: vid.clone(),
                            ltp: (base + wobble * 25) as f64 / 100.0,
                            volume: (mode != TickMode::Ltp)
                                .then(|| 1_234_567 + u64::try_from(step % 1000).unwrap_or(0)),
                            open: Some(base as f64 / 100.0),
                            high: Some((base + 2_500) as f64 / 100.0),
                            low: Some((base - 2_500) as f64 / 100.0),
                            close: Some((base - 800) as f64 / 100.0),
                            exchange_ts_ms: Some(Utc::now().timestamp_millis()),
                        }
                    })
                    .collect();
                if sender.send(TickBatch { ticks }).await.is_err() {
                    // Nobody is listening any more: stop.
                    return;
                }
                step += 1;
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
        Ok(TickSubscription::new(receiver))
    }
}

/// A demo session token: not a secret, and never stored.
pub fn demo_session() -> SessionInfo {
    SessionInfo {
        access_token: SecretString::from("demo"),
        expires_at: demo_expiry(),
        user_id: "demo".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SessionState;

    #[tokio::test]
    async fn the_account_serves_the_demo_data() {
        let account = DemoBroker;
        assert_eq!(account.broker_id(), BrokerId::Demo);
        let session = account.session().await.unwrap();
        assert_eq!(session.state, SessionState::Connected);
        assert_eq!(account.holdings().await.unwrap().len(), 2);
        let positions = account.positions().await.unwrap();
        assert_eq!(positions.net.len(), 2);
        assert_eq!(positions.day.len(), 1);
        let orders = account.orders().await.unwrap();
        assert_eq!(orders.len(), 3);
        assert_eq!(account.trades().await.unwrap().len(), 1);
        let funds = account.funds().await.unwrap();
        assert_eq!(funds.available, 15_234_560);
    }

    #[tokio::test]
    async fn the_stream_walks_prices_and_stops_when_dropped() {
        let stream = DemoStream;
        let ids: Vec<InstrumentId> = BASES.iter().map(|(s, _)| id(s)).collect();
        let mut subscription = stream.subscribe(&ids, TickMode::Quote).await.unwrap();
        let first = subscription.next().await.unwrap();
        assert_eq!(first.ticks.len(), ids.len());
        for tick in &first.ticks {
            assert!(tick.ltp > 0.0 && tick.volume.is_some());
        }
        // The walk moves: two steps differ.
        let second = subscription.next().await.unwrap();
        assert_ne!(first.ticks[0].ltp, second.ticks[0].ltp);

        drop(subscription);
        // The producer task ends; no panic, no leak.
        tokio::time::sleep(Duration::from_millis(1100)).await;
    }

    #[tokio::test]
    async fn ltp_mode_carries_only_the_price() {
        let stream = DemoStream;
        let ids = vec![id("RELIANCE")];
        let mut subscription = stream.subscribe(&ids, TickMode::Ltp).await.unwrap();
        let batch = subscription.next().await.unwrap();
        assert!(batch.ticks[0].volume.is_none());
    }

    #[test]
    fn the_session_expires_at_the_next_open() {
        let expiry = demo_expiry();
        let now = Utc::now();
        assert!(expiry > now);
        assert!(expiry - now <= chrono::Duration::hours(24) + chrono::Duration::hours(9));
    }
}
