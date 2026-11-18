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
use crate::commands::watchlists::WatchlistsDto;
use crate::ipc::IpcError;
use crate::ipc::chart::{BarDto, HistoryDto, IndicatorsDto};
use crate::ipc::dto::{
    HubUpdateDto, IndexSnapshotDto, InstrumentDto, ProviderStatusDto, SourcedIndexQuoteDto,
    SourcedQuoteDto, TopicDto,
};
use crate::ipc::markets::{BreadthDto, DmaBreadthDto, MoversDto, NewsDto, ProgressDto, SectorsDto};

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
    InstrumentDto::export_all(&cfg)?;
    SourcedQuoteDto::export_all(&cfg)?;
    SourcedIndexQuoteDto::export_all(&cfg)?;
    IndexSnapshotDto::export_all(&cfg)?;
    TopicDto::export_all(&cfg)?;
    HubUpdateDto::export_all(&cfg)?;
    ProviderStatusDto::export_all(&cfg)?;
    WatchlistsDto::export_all(&cfg)?;
    HistoryDto::export_all(&cfg)?;
    BarDto::export_all(&cfg)?;
    IndicatorsDto::export_all(&cfg)?;
    MoversDto::export_all(&cfg)?;
    BreadthDto::export_all(&cfg)?;
    DmaBreadthDto::export_all(&cfg)?;
    ProgressDto::export_all(&cfg)?;
    SectorsDto::export_all(&cfg)?;
    NewsDto::export_all(&cfg)?;
    Ok(())
}
