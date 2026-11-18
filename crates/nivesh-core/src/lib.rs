//! Domain types shared by every Nivesh crate.
//!
//! This crate is pure: no I/O, no async, no clocks. Anything that needs the
//! current time takes it as an argument so behaviour is deterministic and
//! testable.

pub mod instrument;

pub use instrument::{Exchange, InstrumentId, InstrumentIdError};
