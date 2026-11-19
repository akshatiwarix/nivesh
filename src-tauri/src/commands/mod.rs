//! Tauri command handlers. Keep them thin: parse arguments, call a service,
//! map the error.
//!
//! Every command must also be listed in `build.rs` (which generates its
//! `allow-*` permission) and granted in `capabilities/main.json`.

pub mod app;
pub mod broker;
pub mod chart;
pub mod data;
pub mod market;
pub mod markets;
pub mod ui_state;
pub mod watchlists;
