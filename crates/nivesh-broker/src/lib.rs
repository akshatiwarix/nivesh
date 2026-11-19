//! Broker adapters for Nivesh: the traits every broker speaks, the account
//! domain types, the shared order state machine and the conformance suite.
//!
//! `BrokerAuth`, `BrokerAccount` and `MarketStream` cover login, account data
//! and live ticks, and cannot trade. Order placement exists only as
//! [`BrokerExecution`], which takes an `ApprovedOrder` minted by the risk
//! engine: no code path below that level can trade.

pub mod conformance;
pub mod demo;
pub mod error;
pub mod status;
pub mod ticks;
pub mod traits;
pub mod types;

pub use demo::{DemoBroker, DemoStream, demo_session};
pub use error::BrokerError;
pub use status::OrderStatus;
pub use ticks::{Tick, TickBatch, TickMode, TickSubscription};
pub use traits::{BrokerAccount, BrokerAuth, BrokerExecution, MarketStream};
pub use types::{
    BrokerId, BrokerOrderId, Funds, Holding, LoginCallback, LoginState, Order, OrderAck, OrderKind,
    OrderTag, Position, Positions, Product, SessionInfo, SessionState, SessionStatus, Side, Trade,
    new_nonce,
};
