use serde::Serialize;
use tauri::AppHandle;
use ts_rs::TS;

use crate::ipc::IpcError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum BuildProfile {
    Debug,
    Release,
}

/// Static facts about the running app, for the status bar and HELP.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub build: BuildProfile,
    pub os: String,
    pub arch: String,
    pub tauri_version: String,
}

#[tauri::command]
pub async fn app_info(app: AppHandle) -> Result<AppInfo, IpcError> {
    let package = app.package_info();
    Ok(AppInfo {
        name: package.name.clone(),
        version: package.version.to_string(),
        build: if cfg!(debug_assertions) {
            BuildProfile::Debug
        } else {
            BuildProfile::Release
        },
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        tauri_version: tauri::VERSION.to_owned(),
    })
}

/// When the process started, for the startup budget (PLAN.md §2.9).
static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
static READY_LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Call first thing in `run`.
pub fn mark_process_start() {
    let _ = STARTED.set(std::time::Instant::now());
}

fn since_start_ms() -> u32 {
    STARTED.get().map_or(0, |t| {
        u32::try_from(t.elapsed().as_millis()).unwrap_or(u32::MAX)
    })
}

/// The UI calls this once its shell has painted and takes keys. Returns
/// the milliseconds from process start to that point; the first call logs
/// it (`startup to interactive`), so release startup can be read from the
/// log.
#[tauri::command]
pub async fn app_ready() -> Result<u32, IpcError> {
    let ms = since_start_ms();
    if !READY_LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        tracing::info!(ms, "startup to interactive");
    }
    Ok(ms)
}

/// Where the app keeps its data and logs, for SETTINGS.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AppPaths {
    pub data_dir: String,
    pub log_dir: String,
}

#[tauri::command]
pub async fn app_paths(app: AppHandle) -> Result<AppPaths, IpcError> {
    use tauri::Manager as _;
    let path = |p: tauri::Result<std::path::PathBuf>| {
        p.map(|p| p.display().to_string())
            .map_err(|e| IpcError::internal(format!("cannot resolve the folder: {e}")))
    };
    Ok(AppPaths {
        data_dir: path(app.path().app_data_dir())?,
        log_dir: path(app.path().app_log_dir())?,
    })
}
