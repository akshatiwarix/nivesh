//! Broker session commands: setup (credentials go Rust → keychain and are
//! never returned), login (URL out, redirect in via a loopback listener),
//! logout and status. The completion and logout paths emit `broker:session`.

use std::sync::Arc;

use tauri::{AppHandle, Emitter as _, State};

use crate::broker::BrokerManager;
use crate::ipc::IpcError;
use crate::ipc::broker::{BrokerLoginDto, BrokerSessionDto};

/// The event the UI listens to for session changes.
pub const SESSION_EVENT: &str = "broker:session";

/// Stores the API credentials and (re)builds the wiring. Returns nothing
/// about the credentials.
#[tauri::command]
pub async fn broker_setup(
    api_key: String,
    api_secret: String,
    broker: State<'_, Arc<BrokerManager>>,
) -> Result<(), IpcError> {
    broker
        .setup(&api_key, &api_secret)
        .await
        .map_err(IpcError::from)
}

/// Starts a login: builds the URL, spawns the one-shot loopback listener,
/// and hands the URL back for the system browser. When the redirect lands,
/// the session is exchanged and `broker:session` fires.
#[tauri::command]
pub async fn broker_login(
    app: AppHandle,
    broker: State<'_, Arc<BrokerManager>>,
) -> Result<BrokerLoginDto, IpcError> {
    let url = broker.begin_login().await.map_err(IpcError::from)?;
    let manager = Arc::clone(&*broker);
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        // The listener lives at most five minutes; either outcome updates
        // the UI through the event.
        let outcome = match manager_kite_listener(&manager).await {
            Ok(callback) => manager.complete_login(callback).await,
            Err(_) => Ok(manager.cancel_login().await),
        };
        if let Ok(session) = outcome {
            let _ = handle.emit(SESSION_EVENT, &session);
        } else if let Err(err) = outcome {
            tracing::warn!(error = %err, "broker login did not complete");
            let _ = handle.emit(SESSION_EVENT, &manager.cancel_login().await);
        }
    });
    Ok(BrokerLoginDto {
        url: url.as_str().to_owned(),
    })
}

/// Waits for the redirect on the registered loopback port.
async fn manager_kite_listener(
    manager: &Arc<BrokerManager>,
) -> Result<nivesh_broker::LoginCallback, IpcError> {
    let wiring = manager
        .kite_wiring()
        .await
        .ok_or_else(|| IpcError::internal("no broker wiring for the login listener"))?;
    wiring
        .login
        .wait_for_redirect()
        .await
        .map_err(IpcError::from)
}

/// Starts the demo broker: the whole account experience with no broker
/// account and no network, clearly labelled demo. `LOGOUT` ends it.
#[tauri::command]
pub async fn broker_demo(
    app: AppHandle,
    broker: State<'_, Arc<BrokerManager>>,
) -> Result<BrokerSessionDto, IpcError> {
    let session = broker.setup_demo().await.map_err(IpcError::from)?;
    let _ = app.emit(SESSION_EVENT, &session);
    Ok(session)
}

/// Ends the session and reports the (now logged-out) status.
#[tauri::command]
pub async fn broker_logout(
    app: AppHandle,
    broker: State<'_, Arc<BrokerManager>>,
) -> Result<BrokerSessionDto, IpcError> {
    let session = broker.logout().await.map_err(IpcError::from)?;
    let _ = app.emit(SESSION_EVENT, &session);
    Ok(session)
}

/// The session as it stands, without a live call to the broker.
#[tauri::command]
pub async fn broker_status(
    broker: State<'_, Arc<BrokerManager>>,
) -> Result<BrokerSessionDto, IpcError> {
    Ok(broker.status().await)
}
