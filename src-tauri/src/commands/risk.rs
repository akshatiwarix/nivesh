//! Safety-control commands: the kill switch. Engaging is immediate;
//! resetting needs the typed phrase, which Rust checks, after a cool-down.

use std::sync::Arc;

use tauri::State;

use crate::ipc::IpcError;
use crate::ipc::risk::KillStatusDto;
use crate::paper::PaperManager;

type Paper<'a> = State<'a, Arc<PaperManager>>;

#[tauri::command]
pub async fn kill_status(paper: Paper<'_>) -> Result<KillStatusDto, IpcError> {
    Ok(KillStatusDto::from(&paper.kill_state()))
}

/// Engages the kill switch: blocks new orders and cancels open ones.
#[tauri::command]
pub async fn kill_engage(paper: Paper<'_>) -> Result<KillStatusDto, IpcError> {
    paper.engage_kill("engaged by the user")?;
    Ok(KillStatusDto::from(&paper.kill_state()))
}

/// Resets the kill switch: `typed` must be the reset phrase.
#[tauri::command]
pub async fn kill_reset(typed: String, paper: Paper<'_>) -> Result<KillStatusDto, IpcError> {
    paper.reset_kill(&typed)?;
    Ok(KillStatusDto::from(&paper.kill_state()))
}
