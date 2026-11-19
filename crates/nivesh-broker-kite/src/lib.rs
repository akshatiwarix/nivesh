//! Zerodha Kite Connect adapter for Nivesh. Part 4 is read-only: login,
//! account data and live ticks. No code in this crate places, changes or
//! cancels an order — that arrives in Part 6 behind the `live-trading`
//! feature and the risk engine.

pub mod account;
pub mod dump;
pub mod kite_account;
pub mod login;
pub mod parser;
pub mod rest;
pub mod stream;

pub use account::parse_order_status;
pub use dump::{KITE as KITE_BROKER, refresh as refresh_instrument_dump};
pub use kite_account::KiteAccount;
pub use login::{
    DEFAULT_REDIRECT_PORT, KEY_ACCESS_TOKEN, KEY_API_KEY, KEY_API_SECRET, KiteLogin, REDIRECT_PATH,
};
pub use parser::RawTick;
pub use rest::{DEFAULT_BASE_URL, KiteRest, RateClass};
pub use stream::{KiteStream, TICKER_URL};
