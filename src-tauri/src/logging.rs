//! Diagnostics logging.
//!
//! `tracing` writes daily-rolling files to the app log directory and, in debug
//! builds, also to stderr. Timestamps are UTC. Override the filter with the
//! `NIVESH_LOG` environment variable (same syntax as `RUST_LOG`).
//!
//! This is the diagnostics log only. The trading audit log (Part 6) is separate
//! and lives in SQLite.

use anyhow::Context as _;
use tauri::{AppHandle, Manager as _};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

const DEFAULT_FILTER: &str = "info,tao=warn,wry=warn";
const MAX_LOG_FILES: usize = 14;

/// Keeps the background log writer alive; dropping it flushes and stops it.
pub struct LogGuard {
    _writer: WorkerGuard,
}

pub fn init(app: &AppHandle) -> anyhow::Result<LogGuard> {
    let dir = app
        .path()
        .app_log_dir()
        .context("resolving app log directory")?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating log directory {}", dir.display()))?;

    let file = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("nivesh")
        .filename_suffix("log")
        .max_log_files(MAX_LOG_FILES)
        .build(&dir)
        .context("creating rolling log file")?;
    let (file_writer, guard) = tracing_appender::non_blocking(file);

    let filter =
        EnvFilter::try_from_env("NIVESH_LOG").unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_writer)
        .with_ansi(false);
    let stderr_layer = cfg!(debug_assertions)
        .then(|| tracing_subscriber::fmt::layer().with_writer(std::io::stderr));

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(stderr_layer)
        .try_init()
        .context("installing tracing subscriber")?;

    tracing::info!(dir = %dir.display(), "logging to file");
    Ok(LogGuard { _writer: guard })
}
