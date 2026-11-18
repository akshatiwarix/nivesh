//! The IPC surface between Rust and the webview.
//!
//! Commands are `async`, return `Result<T, IpcError>`, and live in
//! [`crate::commands`]. Every type that crosses IPC derives `ts_rs::TS` and is
//! listed in [`bindings::export`], which generates `ui/ipc/bindings`.

pub mod bindings;
mod error;

pub use error::{ErrorCode, IpcError};
