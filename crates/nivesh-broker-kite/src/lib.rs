//! Zerodha Kite Connect adapter for Nivesh. Part 4 is read-only: login,
//! account data and live ticks. No code in this crate places, changes or
//! cancels an order — that arrives in Part 6 behind the `live-trading`
//! feature and the risk engine.

pub mod account;
pub mod kite_account;
pub mod login;
pub mod rest;

pub use account::parse_order_status;
pub use kite_account::KiteAccount;
pub use login::{KEY_ACCESS_TOKEN, KEY_API_SECRET, KiteLogin};
pub use rest::{DEFAULT_BASE_URL, KiteRest, RateClass};
