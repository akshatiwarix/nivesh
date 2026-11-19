//! The Kite account mapping and the shared conformance suite, against
//! scrubbed fixture payloads served by a mock Kite server.

use std::sync::Arc;

use nivesh_broker::conformance::{Expectations, account_suite};
use nivesh_broker::{
    BrokerAccount, BrokerError, BrokerId, OrderKind, OrderStatus, Positions, Product, SessionState,
    Side,
};
use nivesh_core::{Exchange, InstrumentId};
use nivesh_secrets::InMemorySecretStore;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use nivesh_broker_kite::{KiteAccount, KiteRest};
use nivesh_secrets::SecretStore as _;
use support::{expected, served_account};

#[cfg(test)]
mod support {
    use super::*;

    pub(crate) const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/kite");

    pub(crate) fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{FIXTURES}/{name}")).unwrap()
    }

    pub(crate) fn body(envelope_data: &str) -> String {
        format!(r#"{{"status":"success","data":{envelope_data}}}"#)
    }

    pub(crate) fn id(symbol: &str) -> InstrumentId {
        InstrumentId::new(Exchange::Nse, symbol).unwrap()
    }

    pub(crate) async fn served_account() -> (MockServer, KiteAccount) {
        let server = MockServer::start().await;
        for (p, file) in [
            ("/user/profile", "profile.json"),
            ("/portfolio/holdings", "holdings.json"),
            ("/portfolio/positions", "positions.json"),
            ("/orders", "orders.json"),
            ("/trades", "trades.json"),
            ("/user/margins", "margins.json"),
        ] {
            Mock::given(method("GET"))
                .and(path(p))
                .respond_with(ResponseTemplate::new(200).set_body_string(body(&fixture(file))))
                .mount(&server)
                .await;
        }
        let rest = Arc::new(KiteRest::new(Some(server.uri()), "testkey".into()).unwrap());
        rest.set_token(secrecy::SecretString::from("day-token"));
        let secrets = Arc::new(InMemorySecretStore::new());
        secrets
            .put(
                nivesh_broker_kite::KEY_ACCESS_TOKEN,
                secrecy::SecretString::from("day-token"),
            )
            .unwrap();
        (server, KiteAccount::new(rest, secrets))
    }

    /// The domain values the fixtures decode to, by hand once — the conformance
    /// suite then proves the adapter agrees.
    pub(crate) fn expected() -> Expectations {
        let expires = nivesh_broker_kite::login::next_token_flush_value();
        Expectations {
            session: nivesh_broker::SessionStatus {
                state: SessionState::Connected,
                expires_at: Some(expires),
            },
            holdings: vec![
                nivesh_broker::Holding {
                    id: id("RELIANCE"),
                    quantity: 10,
                    average_price: 125_025,
                    last_price: Some(137_560),
                    pnl: Some(125_350),
                },
                nivesh_broker::Holding {
                    id: id("TCS"),
                    quantity: 5,
                    average_price: 345_000,
                    last_price: Some(342_100),
                    pnl: Some(-14_500),
                },
            ],
            positions: Positions {
                net: vec![
                    nivesh_broker::Position {
                        id: id("SBIN"),
                        net_quantity: 20,
                        day_quantity: 20,
                        average_price: 81_050,
                        last_price: Some(81_200),
                        pnl: Some(3_000),
                        realised: Some(0),
                    },
                    nivesh_broker::Position {
                        id: id("HDFCBANK"),
                        net_quantity: -5,
                        day_quantity: 0,
                        average_price: 160_000,
                        last_price: Some(162_000),
                        pnl: Some(-10_000),
                        realised: Some(0),
                    },
                ],
                day: vec![nivesh_broker::Position {
                    id: id("SBIN"),
                    net_quantity: 20,
                    day_quantity: 20,
                    average_price: 81_050,
                    last_price: Some(81_200),
                    pnl: Some(3_000),
                    realised: Some(0),
                }],
            },
            orders: vec![
                nivesh_broker::Order {
                    id: nivesh_broker::BrokerOrderId("2511190001".into()),
                    id_instrument: id("SBIN"),
                    side: Side::Buy,
                    kind: OrderKind::Limit,
                    product: Product::Cnc,
                    quantity: 10,
                    filled_quantity: 10,
                    price: Some(81_250),
                    average_price: Some(81_230),
                    status: OrderStatus::Filled,
                    placed_at: Some(
                        chrono::DateTime::parse_from_rfc3339("2026-11-19T04:15:12Z")
                            .unwrap()
                            .into(),
                    ),
                    tag: Some("nivesh-1".into()),
                },
                nivesh_broker::Order {
                    id: nivesh_broker::BrokerOrderId("2511190002".into()),
                    id_instrument: id("TATASTEEL"),
                    side: Side::Sell,
                    kind: OrderKind::StopLoss,
                    product: Product::Mis,
                    quantity: 5,
                    filled_quantity: 2,
                    price: Some(14_500),
                    average_price: None,
                    status: OrderStatus::Open,
                    placed_at: Some(
                        chrono::DateTime::parse_from_rfc3339("2026-11-19T04:32:30Z")
                            .unwrap()
                            .into(),
                    ),
                    tag: None,
                },
                nivesh_broker::Order {
                    id: nivesh_broker::BrokerOrderId("2511190003".into()),
                    id_instrument: id("INFY"),
                    side: Side::Buy,
                    kind: OrderKind::Market,
                    product: Product::Mis,
                    quantity: 1,
                    filled_quantity: 0,
                    price: None,
                    average_price: None,
                    status: OrderStatus::Rejected,
                    placed_at: Some(
                        chrono::DateTime::parse_from_rfc3339("2026-11-19T04:35:00Z")
                            .unwrap()
                            .into(),
                    ),
                    tag: Some("nivesh-2".into()),
                },
            ],
            trades: vec![nivesh_broker::Trade {
                order_id: nivesh_broker::BrokerOrderId("2511190001".into()),
                id_instrument: id("SBIN"),
                side: Side::Buy,
                quantity: 10,
                price: 81_230,
                traded_at: chrono::DateTime::parse_from_rfc3339("2026-11-19T04:15:15Z")
                    .unwrap()
                    .into(),
            }],
            funds: nivesh_broker::Funds {
                available: 15_234_560,
                used: 1_220_000,
                total: 15_234_560,
            },
        }
    }
}

#[tokio::test]
async fn the_kite_adapter_passes_the_account_conformance_suite() {
    let (_server, account) = served_account().await;
    account_suite(&account, &expected()).await.unwrap();
}

/// A stored token the client does not have is a wiring bug, reported loudly.
#[tokio::test]
async fn session_without_a_client_token_is_an_error_not_silently_logged_out() {
    let server = MockServer::start().await;
    let rest = Arc::new(KiteRest::new(Some(server.uri()), "k".into()).unwrap());
    let secrets = Arc::new(InMemorySecretStore::new());
    secrets
        .put(
            nivesh_broker_kite::KEY_ACCESS_TOKEN,
            secrecy::SecretString::from("t"),
        )
        .unwrap();
    let account = KiteAccount::new(rest, secrets);
    let err = account.session().await.unwrap_err();
    assert!(matches!(err, BrokerError::Upstream(text) if text.contains("restore")));
}

/// No token anywhere: logged out.
#[tokio::test]
async fn session_with_no_token_reports_logged_out() {
    let server = MockServer::start().await;
    let rest = Arc::new(KiteRest::new(Some(server.uri()), "k".into()).unwrap());
    let account = KiteAccount::new(rest, Arc::new(InMemorySecretStore::new()));
    let status = account.session().await.unwrap();
    assert_eq!(status.state, SessionState::LoggedOut);
}

/// The broker refusing the token means expired, so the UI asks for a re-login.
#[tokio::test]
async fn a_token_exception_makes_the_session_expired() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/profile"))
        .respond_with(ResponseTemplate::new(403).set_body_string(
            r#"{"status":"error","message":"session expired","error_type":"TokenException"}"#,
        ))
        .mount(&server)
        .await;
    let rest = Arc::new(KiteRest::new(Some(server.uri()), "k".into()).unwrap());
    rest.set_token(secrecy::SecretString::from("dead"));
    let secrets = Arc::new(InMemorySecretStore::new());
    secrets
        .put(
            nivesh_broker_kite::KEY_ACCESS_TOKEN,
            secrecy::SecretString::from("dead"),
        )
        .unwrap();
    let account = KiteAccount::new(rest, secrets);
    let status = account.session().await.unwrap();
    assert_eq!(status.state, SessionState::Expired);
    assert!(status.expires_at.is_none());
}

/// Every Kite order status string maps through the state machine exactly as
/// documented, and the mapping never invents a transition.
#[test]
fn kite_order_statuses_map_onto_the_state_machine() {
    use nivesh_broker::OrderStatus::*;
    let cases: &[(OrderStatus, OrderStatus, bool)] = &[
        (Pending, Open, true),
        (Pending, Filled, true),
        (Pending, Cancelled, true),
        (Pending, Rejected, true),
        (Open, PartiallyFilled, true),
        (Open, Filled, true),
        (Open, Cancelled, true),
        (Open, Rejected, true),
        (PartiallyFilled, Filled, true),
        (PartiallyFilled, Cancelled, true),
        (PartiallyFilled, Rejected, true),
        (Filled, Open, false),
        (Cancelled, Filled, false),
        (Rejected, Pending, false),
    ];
    nivesh_broker::conformance::assert_status_mapping(cases);

    // The Kite string table itself.
    let table = [
        ("OPEN", Open),
        ("TRIGGER PENDING", Open),
        ("PUT ORDER RECEIVED", Pending),
        ("COMPLETE", Filled),
        ("CANCELLED", Cancelled),
        ("REJECTED", Rejected),
    ];
    for (kite, want) in table {
        assert_eq!(nivesh_broker_kite::parse_order_status(kite).unwrap(), want);
    }
    assert!(nivesh_broker_kite::parse_order_status("MYSTERY").is_err());
}

/// The broker id is honest: this adapter says Kite.
#[tokio::test]
async fn the_adapter_identifies_as_kite() {
    let (_server, account) = served_account().await;
    assert_eq!(account.broker_id(), BrokerId::Kite);
}

/// The execution suite against a scripted Kite: placement, reconciliation
/// by tag, cancellation and an expired approval. Only with `live-trading`.
#[cfg(test)]
#[cfg(feature = "live-trading")]
mod execution {
    use super::*;
    use nivesh_broker::conformance::{ExecutionExpectations, execution_suite};
    use nivesh_broker::{BrokerOrderId, OrderTag};
    use nivesh_broker_kite::{ExecutionConfig, KiteExecution};
    use nivesh_risk::{ApprovedOrder, OrderRequest, Product, RiskContext, RiskEngine, RiskLimits};

    fn approval(minted_at: i64) -> ApprovedOrder {
        RiskEngine::new(RiskLimits {
            trading_window: None,
            ..RiskLimits::default()
        })
        .check(
            &OrderRequest {
                symbol: "NSE:RELIANCE".into(),
                side: Side::Buy,
                quantity: 1,
                price: None,
                product: Product::Delivery,
            },
            &RiskContext {
                ltp: 1_375.6,
                position_quantity: 0,
                symbol_exposure_paise: 0,
                open_positions: 0,
                recent_orders: &[],
                day_pnl_paise: 0,
                strategy_limits: None,
                now_ms: minted_at,
            },
        )
        .unwrap()
    }

    #[tokio::test]
    async fn the_kite_execution_passes_the_execution_suite() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/orders/regular"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"status":"success","data":{"order_id":"7001"}}"#),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/orders"))
            .respond_with(ResponseTemplate::new(200).set_body_string(support::body(
                r#"[{"order_id":"7001","exchange":"NSE","tradingsymbol":"RELIANCE",
                    "transaction_type":"BUY","order_type":"LIMIT","product":"CNC",
                    "quantity":1,"filled_quantity":0,"price":1375.6,"average_price":0,
                    "status":"OPEN","order_timestamp":"2026-11-20 10:15:00","tag":"NV3S1"}]"#,
            )))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/orders/regular/7001"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"status":"success","data":{"order_id":"7001"}}"#),
            )
            .expect(1)
            .mount(&server)
            .await;
        let rest = Arc::new(KiteRest::new(Some(server.uri()), "testkey".into()).unwrap());
        rest.set_token(secrecy::SecretString::from("day-token"));
        let secrets = Arc::new(InMemorySecretStore::new());
        secrets
            .put(
                nivesh_broker_kite::KEY_ACCESS_TOKEN,
                secrecy::SecretString::from("day-token"),
            )
            .unwrap();
        let kite = KiteExecution::new(
            KiteAccount::new(Arc::clone(&rest), secrets),
            rest,
            ExecutionConfig::default(),
        );
        let now = nivesh_risk::now_ms();
        execution_suite(
            &kite,
            approval(now),
            approval(now - 10 * 60_000),
            &ExecutionExpectations {
                placed_id: BrokerOrderId("7001".into()),
                tag: OrderTag::new(3, 1).unwrap(),
                unknown_tag: OrderTag::new(3, 2).unwrap(),
            },
        )
        .await
        .unwrap();
    }
}
