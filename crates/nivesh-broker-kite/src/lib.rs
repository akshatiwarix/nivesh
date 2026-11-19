//! Zerodha Kite Connect adapter for Nivesh: login, account data and live
//! ticks. Order placement (`execution`) is compiled only with the
//! `live-trading` feature: a default build cannot send an order to Zerodha.
//! Even then it takes only risk-approved orders, and only
//! `nivesh-exec::OrderGateway` may hold it.

pub mod account;
pub mod dump;
#[cfg(feature = "live-trading")]
pub mod execution;
pub mod kite_account;
pub mod login;
pub mod parser;
pub mod provider;
pub mod rest;
pub mod stream;

pub use account::parse_order_status;
pub use dump::{KITE as KITE_BROKER, refresh as refresh_instrument_dump};
#[cfg(feature = "live-trading")]
pub use execution::{ExecutionConfig, KiteExecution, parse_order_update};
pub use kite_account::KiteAccount;
pub use login::{
    DEFAULT_REDIRECT_PORT, KEY_ACCESS_TOKEN, KEY_API_KEY, KEY_API_SECRET, KiteLogin, REDIRECT_PATH,
};
pub use parser::RawTick;
pub use provider::KiteQuoteProvider;
pub use rest::{DEFAULT_BASE_URL, KiteRest, RateClass};
pub use stream::{KiteStream, TICKER_URL};
