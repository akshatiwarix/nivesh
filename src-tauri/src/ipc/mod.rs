//! The IPC surface between Rust and the webview.
//!
//! Commands are `async`, return `Result<T, IpcError>`, and live in
//! [`crate::commands`]. Every type that crosses IPC derives `ts_rs::TS` and is
//! listed in [`bindings::export`], which generates `ui/ipc/bindings`.

pub mod bindings;
pub mod chart;
pub mod dto;
mod error;
pub mod markets;

pub use error::{ErrorCode, IpcError};
