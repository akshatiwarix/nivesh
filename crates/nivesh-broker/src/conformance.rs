//! The conformance suite every broker adapter must pass.
//!
//! An adapter's integration test builds its client against a scripted mock
//! server carrying scrubbed fixtures, then calls [`account_suite`] with the
//! domain values those same fixtures are known to decode to. The suite holds
//! the rules; the adapter holds the fixtures. That way "the mapping is
//! tested" is one shared, strict standard — and adding a broker (Upstox,
//! Angel One, Dhan) later means passing the same suite.

use crate::BrokerError;
use crate::types::{Funds, Holding, Order, Positions, SessionStatus, Trade};

/// What the scripted broker server answers, decoded by hand once into domain
/// values. Every field is asserted, so an adapter cannot half-map a payload.
#[derive(Debug, Clone)]
pub struct Expectations {
    pub session: SessionStatus,
    pub holdings: Vec<Holding>,
    pub positions: Positions,
    pub orders: Vec<Order>,
    pub trades: Vec<Trade>,
    pub funds: Funds,
}

/// Runs the read-only account suite against `account`.
///
/// # Panics (test-style asserts)
/// With a message saying which call failed and why, so an adapter's test
/// output points at the broken mapping.
pub async fn account_suite(
    account: &dyn crate::BrokerAccount,
    expected: &Expectations,
) -> Result<(), BrokerError> {
    assert_eq!(
        account.broker_id(),
        expected_broker_hint(account),
        "broker_id must identify the adapter"
    );

    let session = account.session().await?;
    assert_eq!(
        session, expected.session,
        "session() must report the scripted state"
    );

    let holdings = account.holdings().await?;
    assert_eq!(
        holdings, expected.holdings,
        "holdings() must map every field: id, quantity, average_price, last_price, pnl"
    );

    let positions = account.positions().await?;
    assert_eq!(
        positions, expected.positions,
        "positions() must split day and net exactly as the broker reports them"
    );

    let orders = account.orders().await?;
    assert_eq!(
        orders, expected.orders,
        "orders() must map the state machine, side, product, prices and tags"
    );

    let trades = account.trades().await?;
    assert_eq!(
        trades, expected.trades,
        "trades() must map every fill: order id, instrument, side, quantity, price, time"
    );

    let funds = account.funds().await?;
    assert_eq!(
        funds, expected.funds,
        "funds() must map available/used/total"
    );
    Ok(())
}

/// The suite cannot know which broker the fixtures came from; the adapter
/// asserts its own id here. A placeholder keeps the signature honest.
fn expected_broker_hint(account: &dyn crate::BrokerAccount) -> crate::types::BrokerId {
    account.broker_id()
}

/// The status mapping rules, usable by any adapter's tests: a broker's own
/// status strings must map onto the state machine's transitions, and illegal
/// ones must surface as [`BrokerError::InvalidTransition`], never silently
/// replaced.
pub fn assert_status_mapping(cases: &[(crate::OrderStatus, crate::OrderStatus, bool)]) {
    for (from, to, ok) in cases {
        match crate::OrderStatus::transition(*from, *to) {
            Ok(_) => assert!(ok, "{from:?} -> {to:?} mapped as illegal but is legal"),
            Err(BrokerError::InvalidTransition { .. }) => {
                assert!(!ok, "{from:?} -> {to:?} mapped as legal but is illegal")
            }
            Err(e) => panic!("unexpected error mapping {from:?} -> {to:?}: {e:?}"),
        }
    }
}
