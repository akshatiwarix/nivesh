//! Order placement on Zerodha Kite: `BrokerExecution`, compiled only with
//! the `live-trading` feature.
//!
//! - **Only approved orders.** `place` and `modify` take an `ApprovedOrder`
//!   from the risk engine, by value, and refuse an expired one before
//!   anything is sent.
//! - **Limit orders by default.** The approved price, rounded to the tick,
//!   is the limit. Market orders are off unless configured, and then always
//!   carry `market_protection` (PLAN.md §11.11).
//! - **Tagged.** Every order carries its `OrderTag`, so an ambiguous outcome
//!   can be looked up in the order book.
//! - **Never retried.** Each request is sent once. A timeout or upstream
//!   failure leaves the order's fate unknown
//!   (`BrokerError::placement_ambiguous`); the caller reconciles by tag.
//! - **Order updates** arrive on the ticker's WebSocket as text frames
//!   ([`parse_order_update`]) and by polling the order book.
//!
//! Regulation (SEBI's retail algo framework, in force for Kite from
//! 1 April 2025): orders through the API must come from the static IP
//! registered with Zerodha, and an unregistered client algo must stay at or
//! under 10 orders per second, where Kite also caps it. The static IP is
//! the operator's setup, documented in `docs/KITE_CONNECT.md`.

use std::sync::Arc;

use async_trait::async_trait;
use nivesh_broker::{
    BrokerAccount, BrokerError, BrokerExecution, BrokerId, BrokerOrderId, Funds, Holding, Order,
    OrderAck, OrderTag, Positions, SessionStatus, Side, Trade,
};
use nivesh_core::InstrumentId;
use nivesh_risk::{ApprovedOrder, Product};
use reqwest::Method;
use serde_json::Value;

use crate::{KiteAccount, KiteRest};

/// How Nivesh's Kite orders are shaped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExecutionConfig {
    /// Send market orders instead of limits. Off by default.
    pub market_orders: bool,
    /// The market protection on a market order, percent (`2.0` = 2%).
    pub market_protection_pct: f64,
    /// The price tick, paise. NSE equity ticks are 5 paise for most stocks.
    pub tick_paise: i64,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            market_orders: false,
            market_protection_pct: 2.0,
            tick_paise: 5,
        }
    }
}

/// The Kite account, able to trade.
pub struct KiteExecution {
    account: KiteAccount,
    rest: Arc<KiteRest>,
    config: ExecutionConfig,
}

impl KiteExecution {
    pub fn new(account: KiteAccount, rest: Arc<KiteRest>, config: ExecutionConfig) -> Self {
        Self {
            account,
            rest,
            config,
        }
    }

    /// The form fields for an approved order (without the tag).
    fn order_fields(
        &self,
        order: &ApprovedOrder,
    ) -> Result<Vec<(&'static str, String)>, BrokerError> {
        let id: InstrumentId = order
            .symbol()
            .parse()
            .map_err(|e| BrokerError::InvalidRequest(format!("symbol {}: {e}", order.symbol())))?;
        let mut fields = vec![
            ("exchange", id.exchange().to_string()),
            ("tradingsymbol", id.symbol().to_owned()),
            (
                "transaction_type",
                match order.side() {
                    Side::Buy => "BUY",
                    Side::Sell => "SELL",
                }
                .to_owned(),
            ),
            ("quantity", order.quantity().to_string()),
            (
                "product",
                match order.product() {
                    Product::Delivery => "CNC",
                    Product::Intraday => "MIS",
                }
                .to_owned(),
            ),
            ("validity", "DAY".to_owned()),
        ];
        if self.config.market_orders {
            if !(self.config.market_protection_pct > 0.0
                && self.config.market_protection_pct <= 100.0)
            {
                return Err(BrokerError::InvalidRequest(
                    "a market order needs market protection between 0 and 100%".into(),
                ));
            }
            fields.push(("order_type", "MARKET".to_owned()));
            fields.push((
                "market_protection",
                format!("{}", self.config.market_protection_pct),
            ));
        } else {
            let price = order
                .price()
                .ok_or_else(|| BrokerError::InvalidRequest("a limit order needs a price".into()))?;
            fields.push(("order_type", "LIMIT".to_owned()));
            fields.push(("price", limit_price(price, self.config.tick_paise)));
        }
        Ok(fields)
    }
}

/// The limit price on the tick, as Kite wants it (`"1000.05"`).
fn limit_price(rupees: f64, tick_paise: i64) -> String {
    let tick = tick_paise.max(1);
    let paise = (rupees * 100.0).round() as i64;
    let on_tick = ((paise + tick / 2) / tick) * tick;
    format!("{}.{:02}", on_tick / 100, on_tick % 100)
}

fn now_ms() -> i64 {
    nivesh_risk::now_ms()
}

fn ack(data: &Value) -> Result<OrderAck, BrokerError> {
    let id = data
        .get("order_id")
        .and_then(|v| {
            v.as_str()
                .map(str::to_owned)
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        })
        .ok_or_else(|| BrokerError::Upstream("an order reply without an order_id".into()))?;
    Ok(OrderAck {
        id: BrokerOrderId(id),
    })
}

#[async_trait]
impl BrokerAccount for KiteExecution {
    fn broker_id(&self) -> BrokerId {
        self.account.broker_id()
    }
    async fn session(&self) -> Result<SessionStatus, BrokerError> {
        self.account.session().await
    }
    async fn holdings(&self) -> Result<Vec<Holding>, BrokerError> {
        self.account.holdings().await
    }
    async fn positions(&self) -> Result<Positions, BrokerError> {
        self.account.positions().await
    }
    async fn orders(&self) -> Result<Vec<Order>, BrokerError> {
        self.account.orders().await
    }
    async fn trades(&self) -> Result<Vec<Trade>, BrokerError> {
        self.account.trades().await
    }
    async fn funds(&self) -> Result<Funds, BrokerError> {
        self.account.funds().await
    }
}

#[async_trait]
impl BrokerExecution for KiteExecution {
    async fn place(&self, order: ApprovedOrder, tag: &OrderTag) -> Result<OrderAck, BrokerError> {
        if !order.is_valid_at(now_ms()) {
            return Err(BrokerError::InvalidRequest(
                "the approval expired; the order must be checked again".into(),
            ));
        }
        let mut fields = self.order_fields(&order)?;
        fields.push(("tag", tag.as_str().to_owned()));
        let data = self
            .rest
            .order_request(Method::POST, "/orders/regular", &fields)
            .await?;
        ack(&data)
    }

    async fn modify(
        &self,
        id: &BrokerOrderId,
        order: ApprovedOrder,
    ) -> Result<OrderAck, BrokerError> {
        if !order.is_valid_at(now_ms()) {
            return Err(BrokerError::InvalidRequest(
                "the approval expired; the change must be checked again".into(),
            ));
        }
        // Kite modifies order type, quantity, price and validity only.
        let fields: Vec<(&str, String)> = self
            .order_fields(&order)?
            .into_iter()
            .filter(|(k, _)| {
                matches!(
                    *k,
                    "order_type" | "quantity" | "price" | "validity" | "market_protection"
                )
            })
            .collect();
        let data = self
            .rest
            .order_request(Method::PUT, &format!("/orders/regular/{}", id.0), &fields)
            .await?;
        ack(&data)
    }

    async fn cancel(&self, id: &BrokerOrderId) -> Result<OrderAck, BrokerError> {
        let data = self
            .rest
            .order_request(Method::DELETE, &format!("/orders/regular/{}", id.0), &[])
            .await?;
        ack(&data)
    }
}

/// An order update from the ticker's WebSocket: a text frame
/// `{"type": "order", "data": {...}}` carrying an order-book entry. Other
/// text frames (errors, messages) are `None`.
pub fn parse_order_update(text: &str) -> Option<Order> {
    let frame: Value = serde_json::from_str(text).ok()?;
    if frame.get("type")?.as_str()? != "order" {
        return None;
    }
    crate::account::order(frame.get("data")?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nivesh_broker::OrderStatus;
    use nivesh_risk::{OrderRequest, RiskContext, RiskEngine, RiskLimits};
    use nivesh_secrets::{InMemorySecretStore, SecretStore as _};
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn approved(side: Side, qty: i64, ltp: f64) -> ApprovedOrder {
        let engine = RiskEngine::new(RiskLimits {
            trading_window: None,
            ..RiskLimits::default()
        });
        let ctx = RiskContext {
            ltp,
            position_quantity: if side == Side::Sell { qty } else { 0 },
            symbol_exposure_paise: 0,
            open_positions: 0,
            recent_orders: &[],
            day_pnl_paise: 0,
            strategy_limits: None,
            now_ms: now_ms(),
        };
        engine
            .check(
                &OrderRequest {
                    symbol: "NSE:RELIANCE".into(),
                    side,
                    quantity: qty,
                    price: None,
                    product: Product::Delivery,
                },
                &ctx,
            )
            .unwrap()
    }

    /// An approval minted ten minutes ago: expired.
    fn stale() -> ApprovedOrder {
        let engine = RiskEngine::new(RiskLimits {
            trading_window: None,
            ..RiskLimits::default()
        });
        let ctx = RiskContext {
            ltp: 1_000.0,
            position_quantity: 0,
            symbol_exposure_paise: 0,
            open_positions: 0,
            recent_orders: &[],
            day_pnl_paise: 0,
            strategy_limits: None,
            now_ms: now_ms() - 10 * 60_000,
        };
        engine
            .check(
                &OrderRequest {
                    symbol: "NSE:RELIANCE".into(),
                    side: Side::Buy,
                    quantity: 1,
                    price: None,
                    product: Product::Delivery,
                },
                &ctx,
            )
            .unwrap()
    }

    fn broker(server: &MockServer, config: ExecutionConfig) -> KiteExecution {
        let rest = Arc::new(KiteRest::new(Some(server.uri()), "testkey".into()).unwrap());
        rest.set_token(secrecy::SecretString::from("testtoken"));
        let secrets = Arc::new(InMemorySecretStore::new());
        secrets
            .put(
                crate::KEY_ACCESS_TOKEN,
                secrecy::SecretString::from("testtoken"),
            )
            .unwrap();
        KiteExecution::new(KiteAccount::new(Arc::clone(&rest), secrets), rest, config)
    }

    fn ok(order_id: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_string(format!(
            r#"{{"status":"success","data":{{"order_id":"{order_id}"}}}}"#
        ))
    }

    #[test]
    fn limit_prices_land_on_the_tick() {
        assert_eq!(limit_price(1_000.0, 5), "1000.00");
        assert_eq!(limit_price(1_000.024, 5), "1000.00");
        assert_eq!(limit_price(1_000.026, 5), "1000.05");
        assert_eq!(limit_price(99.99, 5), "100.00");
        assert_eq!(limit_price(12.34, 1), "12.34");
    }

    #[tokio::test]
    async fn an_approved_order_is_placed_as_a_tagged_limit_order() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/orders/regular"))
            .and(header("X-Kite-Version", "3"))
            .and(header("Authorization", "token testkey:testtoken"))
            .and(body_string_contains("exchange=NSE"))
            .and(body_string_contains("tradingsymbol=RELIANCE"))
            .and(body_string_contains("transaction_type=BUY"))
            .and(body_string_contains("quantity=3"))
            .and(body_string_contains("product=CNC"))
            .and(body_string_contains("validity=DAY"))
            .and(body_string_contains("order_type=LIMIT"))
            .and(body_string_contains("price=1234.55"))
            .and(body_string_contains("tag=NV7S1"))
            .respond_with(ok("250101000000001"))
            .expect(1)
            .mount(&server)
            .await;
        let kite = broker(&server, ExecutionConfig::default());
        let ack = kite
            .place(
                approved(Side::Buy, 3, 1_234.56),
                &OrderTag::new(7, 1).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ack.id, BrokerOrderId("250101000000001".into()));
    }

    #[tokio::test]
    async fn a_market_order_always_carries_market_protection() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/orders/regular"))
            .and(body_string_contains("order_type=MARKET"))
            .and(body_string_contains("market_protection=2"))
            .respond_with(ok("1"))
            .expect(1)
            .mount(&server)
            .await;
        let kite = broker(
            &server,
            ExecutionConfig {
                market_orders: true,
                ..ExecutionConfig::default()
            },
        );
        kite.place(approved(Side::Buy, 1, 100.0), &OrderTag::new(1, 1).unwrap())
            .await
            .unwrap();

        // Without protection it is refused before anything is sent.
        let unprotected = broker(
            &server,
            ExecutionConfig {
                market_orders: true,
                market_protection_pct: 0.0,
                ..ExecutionConfig::default()
            },
        );
        let err = unprotected
            .place(approved(Side::Buy, 1, 100.0), &OrderTag::new(1, 2).unwrap())
            .await
            .unwrap_err();
        assert!(matches!(err, BrokerError::InvalidRequest(_)));
    }

    #[tokio::test]
    async fn an_expired_approval_is_never_sent() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ok("1"))
            .expect(0)
            .mount(&server)
            .await;
        let kite = broker(&server, ExecutionConfig::default());
        let err = kite
            .place(stale(), &OrderTag::new(1, 1).unwrap())
            .await
            .unwrap_err();
        assert!(matches!(err, BrokerError::InvalidRequest(m) if m.contains("expired")));
    }

    #[tokio::test]
    async fn a_rejection_is_definite_and_not_ambiguous() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_string(
                r#"{"status":"error","message":"Insufficient funds","error_type":"MarginException"}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;
        let err = broker(&server, ExecutionConfig::default())
            .place(approved(Side::Buy, 1, 100.0), &OrderTag::new(1, 1).unwrap())
            .await
            .unwrap_err();
        assert!(matches!(&err, BrokerError::InvalidRequest(m) if m.contains("MarginException")));
        assert!(!err.placement_ambiguous());
    }

    #[tokio::test]
    async fn an_upstream_failure_is_ambiguous_sent_once_and_found_by_tag() {
        let server = MockServer::start().await;
        // The OMS fails *after* taking the order: a 502, but the order exists.
        Mock::given(method("POST"))
            .and(path("/orders/regular"))
            .respond_with(ResponseTemplate::new(502).set_body_string(
                r#"{"status":"error","message":"OMS unavailable","error_type":"NetworkException"}"#,
            ))
            .expect(1) // never retried
            .mount(&server)
            .await;
        let book = r#"{"status":"success","data":[{
            "order_id":"9001","exchange":"NSE","tradingsymbol":"RELIANCE",
            "transaction_type":"BUY","order_type":"LIMIT","product":"CNC",
            "quantity":1,"filled_quantity":0,"price":100.0,"average_price":0,
            "status":"OPEN","order_timestamp":"2026-11-20 10:15:00","tag":"NV1S1"}]}"#;
        Mock::given(method("GET"))
            .and(path("/orders"))
            .respond_with(ResponseTemplate::new(200).set_body_string(book))
            .mount(&server)
            .await;
        let kite = broker(&server, ExecutionConfig::default());
        let tag = OrderTag::new(1, 1).unwrap();
        let err = kite
            .place(approved(Side::Buy, 1, 100.0), &tag)
            .await
            .unwrap_err();
        assert!(err.placement_ambiguous(), "{err:?}");
        let found = kite.find_by_tag(&tag).await.unwrap().expect("reconciled");
        assert_eq!(found.id, BrokerOrderId("9001".into()));
        assert_eq!(found.status, OrderStatus::Open);
        assert!(
            kite.find_by_tag(&OrderTag::new(1, 2).unwrap())
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn an_expired_session_is_unauthorised_not_ambiguous() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(403).set_body_string(
                r#"{"status":"error","message":"Session expired","error_type":"TokenException"}"#,
            ))
            .mount(&server)
            .await;
        let err = broker(&server, ExecutionConfig::default())
            .place(approved(Side::Buy, 1, 100.0), &OrderTag::new(1, 1).unwrap())
            .await
            .unwrap_err();
        assert_eq!(err, BrokerError::Unauthorized);
        assert!(!err.placement_ambiguous());
    }

    #[tokio::test]
    async fn modify_and_cancel_hit_the_order() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/orders/regular/9001"))
            .and(body_string_contains("price=101.00"))
            .and(body_string_contains("quantity=2"))
            .respond_with(ok("9001"))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/orders/regular/9001"))
            .respond_with(ok("9001"))
            .expect(1)
            .mount(&server)
            .await;
        let kite = broker(&server, ExecutionConfig::default());
        let id = BrokerOrderId("9001".into());
        assert_eq!(
            kite.modify(&id, approved(Side::Buy, 2, 101.0))
                .await
                .unwrap()
                .id,
            id
        );
        assert_eq!(kite.cancel(&id).await.unwrap().id, id);
    }

    #[test]
    fn order_updates_from_the_websocket_map_onto_orders() {
        let text = r#"{"type":"order","data":{
            "order_id":"9001","exchange":"NSE","tradingsymbol":"RELIANCE",
            "transaction_type":"SELL","order_type":"LIMIT","product":"CNC",
            "quantity":5,"filled_quantity":5,"price":101.5,"average_price":101.5,
            "status":"COMPLETE","order_timestamp":"2026-11-20 10:15:00","tag":"NV1S2"}}"#;
        let order = parse_order_update(text).unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
        assert_eq!(order.filled_quantity, 5);
        assert_eq!(order.average_price, Some(10_150));
        assert_eq!(order.tag.as_deref(), Some("NV1S2"));
        assert!(parse_order_update(r#"{"type":"error","data":"x"}"#).is_none());
        assert!(parse_order_update("not json").is_none());
    }
}
