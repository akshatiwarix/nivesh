//! Broker adapters for Nivesh: the traits every broker speaks, the account
//! domain types, the shared order state machine and the conformance suite.
//!
//! Part 4 is deliberately **read-only**: `BrokerAuth`, `BrokerAccount` and
//! `MarketStream` cover login, account data and live ticks. Order placement
//! arrives in Part 5 as [`crate::traits`] `BrokerExecution`, which takes an
//! `ApprovedOrder` minted by the risk engine — no code path below that level
//! can trade.

pub mod conformance;
pub mod error;
pub mod status;
pub mod ticks;
pub mod traits;
pub mod types;

pub use error::BrokerError;
pub use status::OrderStatus;
pub use ticks::{Tick, TickBatch, TickMode, TickSubscription};
pub use traits::{BrokerAccount, BrokerAuth, MarketStream};
pub use types::{
    BrokerId, BrokerOrderId, Funds, Holding, LoginCallback, LoginState, Order, OrderKind, Position,
    Positions, Product, SessionInfo, SessionState, SessionStatus, Side, Trade,
};
