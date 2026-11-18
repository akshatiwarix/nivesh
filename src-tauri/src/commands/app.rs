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
