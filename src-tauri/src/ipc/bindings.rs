//! TypeScript bindings for IPC types.
//!
//! `pnpm gen:bindings` runs the `gen_bindings` example, which calls [`export`]
//! to rewrite `ui/ipc/bindings`. The `bindings` integration test exports into a
//! scratch directory and fails if the committed files differ.

use std::path::{Path, PathBuf};

use ts_rs::{Config, ExportError, TS};

use crate::commands::app::AppInfo;
use crate::commands::market::MarketStatusDto;
use crate::commands::ui_state::UiState;
use crate::ipc::IpcError;

/// The committed bindings directory.
pub fn committed_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui/ipc/bindings")
}

/// Writes bindings for every IPC type, plus the types they depend on, into
/// `dir` (one `<Type>.ts` file each).
pub fn export(dir: &Path) -> Result<(), ExportError> {
    // serde writes i64/u64 as JSON numbers, so they must not become `bigint`.
    // Integer paise stay far below 2^53.
    let cfg = Config::new().with_out_dir(dir).with_large_int("number");

    IpcError::export_all(&cfg)?;
    AppInfo::export_all(&cfg)?;
    MarketStatusDto::export_all(&cfg)?;
    UiState::export_all(&cfg)?;
    Ok(())
}
