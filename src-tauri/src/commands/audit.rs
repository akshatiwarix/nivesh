//! The audit log, read-only: list (filtered), verify the chain, export.
//! Nothing here can change or delete a record.

use std::collections::HashMap;
use std::sync::Arc;

use nivesh_store::{AuditQuery, Store};
use tauri::{AppHandle, Manager as _, State};

use crate::commands::strategy::strategy_named;
use crate::ipc::IpcError;
use crate::ipc::audit::{AuditRowDto, AuditVerifyDto};

/// Most rows one call returns.
const MAX_ROWS: usize = 5_000;

/// The newest records first. `kind`: a prefix (`order`, `kill.`, ...);
/// `strategy`: a strategy's name.
#[tauri::command]
pub async fn audit_log(
    kind: Option<String>,
    strategy: Option<String>,
    since_ms: Option<i64>,
    limit: Option<u32>,
    store: State<'_, Arc<Store>>,
) -> Result<Vec<AuditRowDto>, IpcError> {
    let strategy_id = match &strategy {
        Some(name) => Some(strategy_named(&store, name)?.id),
        None => None,
    };
    let query = AuditQuery {
        kind_prefix: kind.map(|k| k.to_ascii_lowercase()),
        strategy_id,
        since_ms,
        limit: Some(limit.map_or(MAX_ROWS, |l| (l as usize).min(MAX_ROWS))),
    };
    let names: HashMap<i64, String> = store
        .strategies()?
        .into_iter()
        .map(|r| (r.id, r.name))
        .collect();
    Ok(store
        .audit(&query)?
        .into_iter()
        .map(|r| AuditRowDto {
            seq: r.seq,
            at: r.at_ms,
            kind: r.kind,
            strategy: r.strategy_id.and_then(|id| names.get(&id).cloned()),
            strategy_id: r.strategy_id,
            payload: r.payload,
            hash: r.hash,
        })
        .collect())
}

/// Re-checks the whole hash chain.
#[tauri::command]
pub async fn audit_verify(store: State<'_, Arc<Store>>) -> Result<AuditVerifyDto, IpcError> {
    Ok(AuditVerifyDto::from(store.verify_audit()?))
}

/// Writes the whole log (oldest first) to the Downloads folder as `csv` or
/// `jsonl`. Returns the file's path.
#[tauri::command]
pub async fn audit_export(
    format: String,
    app: AppHandle,
    store: State<'_, Arc<Store>>,
) -> Result<String, IpcError> {
    let (body, ext) = match format.to_ascii_lowercase().as_str() {
        "csv" => (store.export_audit_csv()?, "csv"),
        "jsonl" => (store.export_audit_jsonl()?, "jsonl"),
        other => {
            return Err(IpcError::invalid_argument(format!(
                "export as CSV or JSONL, not {other}"
            )));
        }
    };
    let dir = app
        .path()
        .download_dir()
        .or_else(|_| app.path().app_data_dir())
        .map_err(|e| IpcError::internal(format!("no folder to export to: {e}")))?;
    let name = format!(
        "nivesh-audit-{}.{ext}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    );
    let path = dir.join(name);
    std::fs::write(&path, body)
        .map_err(|e| IpcError::internal(format!("cannot write {}: {e}", path.display())))?;
    tracing::info!(path = %path.display(), "audit log exported");
    Ok(path.display().to_string())
}
