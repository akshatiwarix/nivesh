//! Script strategies: check, save and load their source. Running them goes
//! through the ordinary BT and PAPER commands.

use std::sync::Arc;

use nivesh_store::Store;
use tauri::State;

use crate::commands::strategy::strategy_named;
use crate::ipc::IpcError;
use crate::ipc::strategy::ScriptCheckDto;
use crate::paper::PaperManager;
use crate::strategy_body::{SCRIPT_TEMPLATE, ScriptJson};

/// The largest script accepted.
const MAX_SOURCE: usize = 64 * 1024;

fn check(source: &str) -> ScriptCheckDto {
    if source.len() > MAX_SOURCE {
        return ScriptCheckDto {
            ok: false,
            line: None,
            message: Some(format!("a script can be at most {} KiB", MAX_SOURCE / 1024)),
        };
    }
    ScriptCheckDto::of(nivesh_script::validate(source))
}

/// Compiles `source` in the sandbox without saving it.
#[tauri::command]
pub async fn script_validate(source: String) -> Result<ScriptCheckDto, IpcError> {
    Ok(check(&source))
}

/// Saves a script strategy (new, or replacing the named script). Refused
/// when it does not compile, when the name belongs to a rule strategy, or
/// while it is running: stop it first, so a live or paper run never
/// changes code under it.
#[tauri::command]
pub async fn script_save(
    name: String,
    source: String,
    store: State<'_, Arc<Store>>,
    paper: State<'_, Arc<PaperManager>>,
) -> Result<ScriptCheckDto, IpcError> {
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err(IpcError::invalid_argument("a script needs a name"));
    }
    let checked = check(&source);
    if !checked.ok {
        return Ok(checked);
    }
    let existing = store
        .strategies()?
        .into_iter()
        .find(|r| r.name.eq_ignore_ascii_case(&name));
    if let Some(row) = &existing {
        if row.template != SCRIPT_TEMPLATE {
            return Err(IpcError::invalid_argument(format!(
                "{name} is a {} strategy, not a script; pick another name",
                row.template
            )));
        }
        if paper.is_running(row.id) {
            return Err(IpcError::invalid_argument(format!(
                "{name} is running; PAPER STOP {name} before changing its code"
            )));
        }
    }
    let json = serde_json::to_string(&ScriptJson { source })
        .map_err(|e| IpcError::internal(format!("cannot encode the script: {e}")))?;
    store.save_strategy(
        existing.map(|r| r.id),
        &name,
        SCRIPT_TEMPLATE,
        &json,
        nivesh_risk::now_ms(),
    )?;
    Ok(checked)
}

/// A saved script's source; `None` when there is no strategy by that name.
#[tauri::command]
pub async fn script_source(
    name: String,
    store: State<'_, Arc<Store>>,
) -> Result<Option<String>, IpcError> {
    let Ok(row) = strategy_named(&store, &name) else {
        return Ok(None);
    };
    if row.template != SCRIPT_TEMPLATE {
        return Err(IpcError::invalid_argument(format!(
            "{name} is a {} strategy, not a script",
            row.template
        )));
    }
    let script: ScriptJson = serde_json::from_str(&row.json)
        .map_err(|e| IpcError::internal(format!("stored script is unreadable: {e}")))?;
    Ok(Some(script.source))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_report_the_line() {
        assert!(check("fn on_bar(bar) { }").ok);
        let bad = check("fn on_bar(bar) {\n  let x = ;\n}");
        assert!(!bad.ok);
        assert_eq!(bad.line, Some(2));
        let huge = "x".repeat(MAX_SOURCE + 1);
        assert!(!check(&huge).ok);
    }
}
