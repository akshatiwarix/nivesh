//! The Nivesh SQLite store.
//!
//! One database file in the app data directory, in WAL mode, with versioned
//! migrations. Part 1 persists UI settings, command history and window
//! geometry; Part 2 adds the data cache, instrument master and watchlists.
//! Later parts add tables through new migrations.
//!
//! All access goes through [`Store`], which serialises use of the single
//! connection behind a mutex. Queries here are tiny, so callers on an async
//! runtime may call them directly.
//!
//! Secrets never go in this database (they live in the OS keychain).

mod cache;
mod error;
mod migrations;
mod store;

pub use cache::CacheRow;
pub use error::StoreError;
pub use migrations::LATEST_VERSION;
pub use store::{Store, WindowState};
