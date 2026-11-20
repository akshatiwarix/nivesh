use std::sync::Arc;

use nivesh_store::Store;
use serde::Serialize;
use tauri::State;
use ts_rs::TS;

use crate::ipc::IpcError;

const LAYOUT_KEY: &str = "ui.layout";
/// Set once the first-run onboarding has been read.
const ONBOARDED_KEY: &str = "ui.onboarded";
const VALID_LAYOUTS: [u8; 3] = [1, 2, 4];
const DEFAULT_LAYOUT: u8 = 1;
/// Matches the UI's history bound.
pub const MAX_HISTORY: usize = 100;
const MAX_COMMAND_LEN: usize = 256;

/// What the UI restores on start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UiState {
    /// Number of panels: 1, 2 or 4.
    pub layout: u8,
    /// Command history, oldest first.
    pub history: Vec<String>,
    /// Whether the first-run onboarding has been read.
    pub onboarded: bool,
}

/// A stored layout is trusted only if it is still a valid one.
fn layout_from_setting(value: Option<&str>) -> u8 {
    value
        .and_then(|v| v.parse::<u8>().ok())
        .filter(|l| VALID_LAYOUTS.contains(l))
        .unwrap_or(DEFAULT_LAYOUT)
}

fn validate_history(history: &[String]) -> Result<(), IpcError> {
    if history.len() > MAX_HISTORY {
        return Err(IpcError::invalid_argument(format!(
            "history is limited to {MAX_HISTORY} entries"
        )));
    }
    if history.iter().any(|c| c.len() > MAX_COMMAND_LEN) {
        return Err(IpcError::invalid_argument(format!(
            "history entries are limited to {MAX_COMMAND_LEN} bytes"
        )));
    }
    Ok(())
}

#[tauri::command]
pub async fn ui_state(store: State<'_, Arc<Store>>) -> Result<UiState, IpcError> {
    Ok(UiState {
        layout: layout_from_setting(store.setting(LAYOUT_KEY)?.as_deref()),
        history: store.command_history(MAX_HISTORY)?,
        onboarded: store.setting(ONBOARDED_KEY)?.is_some(),
    })
}

/// Records that the first-run onboarding was read.
#[tauri::command]
pub async fn finish_onboarding(store: State<'_, Arc<Store>>) -> Result<(), IpcError> {
    Ok(store.set_setting(ONBOARDED_KEY, &nivesh_risk::now_ms().to_string())?)
}

#[tauri::command]
pub async fn save_layout(layout: u8, store: State<'_, Arc<Store>>) -> Result<(), IpcError> {
    if !VALID_LAYOUTS.contains(&layout) {
        return Err(IpcError::invalid_argument("layout must be 1, 2 or 4"));
    }
    Ok(store.set_setting(LAYOUT_KEY, &layout.to_string())?)
}

#[tauri::command]
pub async fn save_command_history(
    history: Vec<String>,
    store: State<'_, Arc<Store>>,
) -> Result<(), IpcError> {
    validate_history(&history)?;
    Ok(store.replace_command_history(&history)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_setting_falls_back_to_one_panel() {
        assert_eq!(layout_from_setting(None), 1);
        assert_eq!(layout_from_setting(Some("2")), 2);
        assert_eq!(layout_from_setting(Some("4")), 4);
        for bad in ["3", "0", "-1", "256", "x", ""] {
            assert_eq!(layout_from_setting(Some(bad)), 1, "{bad:?}");
        }
    }

    #[test]
    fn history_limits_are_enforced() {
        assert!(validate_history(&vec!["A".to_owned(); MAX_HISTORY]).is_ok());
        assert!(validate_history(&vec!["A".to_owned(); MAX_HISTORY + 1]).is_err());
        assert!(validate_history(&["x".repeat(MAX_COMMAND_LEN + 1)]).is_err());
    }
}
