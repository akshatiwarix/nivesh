//! Broker session commands: setup (credentials go Rust → keychain and are
//! never returned), login (URL out, redirect in via a loopback listener),
//! logout and status. The completion and logout paths emit `broker:session`.

use std::sync::Arc;

use tauri::{AppHandle, Emitter as _, State};

use crate::broker::BrokerManager;
use crate::ipc::IpcError;
use crate::ipc::broker::{BrokerLoginDto, BrokerSessionDto};
use crate::services::Services;

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
    services: State<'_, Arc<Services>>,
) -> Result<BrokerLoginDto, IpcError> {
    let url = broker.begin_login().await.map_err(IpcError::from)?;
    let manager = Arc::clone(&*broker);
    let services = Arc::clone(&*services);
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        // The listener lives at most five minutes; either outcome updates
        // the UI through the event.
        let outcome = match manager_kite_listener(&manager).await {
            Ok(callback) => manager.complete_login(callback).await,
            Err(_) => Ok(manager.cancel_login().await),
        };
        match outcome {
            Ok(session) => {
                // The broker's live quotes join the chain at the front of
                // the routing order; every value they serve is LIVE.
                if let Some(wiring) = manager.kite_wiring().await {
                    services
                        .market
                        .register(Arc::new(nivesh_broker_kite::KiteQuoteProvider::new(
                            wiring.rest,
                        )));
                }
                let _ = handle.emit(SESSION_EVENT, &session);
            }
            Err(err) => {
                tracing::warn!(error = %err, "broker login did not complete");
                let _ = handle.emit(SESSION_EVENT, &manager.cancel_login().await);
            }
        }
    });
    // The URL is the app's own login page for its own API key: safe to open
    // in the system browser directly.
    if let Err(err) = tauri_plugin_opener::open_url(url.as_str(), None::<&str>) {
        tracing::warn!(error = %err, "could not open the browser for the login");
    }
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
    services: State<'_, Arc<Services>>,
) -> Result<BrokerSessionDto, IpcError> {
    let session = broker.logout().await.map_err(IpcError::from)?;
    // The live provider leaves the chain; quotes fall back to the public
    // sources and their DELAYED provenance.
    services.market.unregister(nivesh_core::ProviderId::Kite);
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

// ---- Account data -----------------------------------------------------------

use crate::ipc::broker::{FundsDto, HoldingDto, OrderDto, PositionsDto, TradeDto};

/// The account data source: the demo broker when it is active, else Kite.
async fn account(
    broker: &Arc<BrokerManager>,
) -> Result<(String, Arc<dyn nivesh_broker::BrokerAccount>), IpcError> {
    if let Some((_demo, _stream)) = broker.demo_wiring().await {
        let account: Arc<dyn nivesh_broker::BrokerAccount> =
            Arc::new(nivesh_broker::DemoBroker) as _;
        return Ok(("demo".to_owned(), account));
    }
    if let Some(wiring) = broker.kite_wiring().await {
        let account: Arc<dyn nivesh_broker::BrokerAccount> = wiring.account.clone();
        return Ok(("kite".to_owned(), account));
    }
    Err(IpcError::new(
        crate::ipc::ErrorCode::Unavailable,
        "No broker is connected. LOGIN first, or LOGIN DEMO for the demo.",
    ))
}

#[tauri::command]
pub async fn broker_holdings(
    broker: State<'_, Arc<BrokerManager>>,
) -> Result<Vec<HoldingDto>, IpcError> {
    let (name, account) = account(&broker).await?;
    let _ = name;
    Ok(account
        .holdings()
        .await
        .map_err(IpcError::from)?
        .iter()
        .map(Into::into)
        .collect())
}

#[tauri::command]
pub async fn broker_positions(
    broker: State<'_, Arc<BrokerManager>>,
) -> Result<PositionsDto, IpcError> {
    let (_name, account) = account(&broker).await?;
    Ok(PositionsDto::from(
        &account.positions().await.map_err(IpcError::from)?,
    ))
}

#[tauri::command]
pub async fn broker_orders(
    broker: State<'_, Arc<BrokerManager>>,
) -> Result<Vec<OrderDto>, IpcError> {
    let (_name, account) = account(&broker).await?;
    Ok(account
        .orders()
        .await
        .map_err(IpcError::from)?
        .iter()
        .map(Into::into)
        .collect())
}

#[tauri::command]
pub async fn broker_trades(
    broker: State<'_, Arc<BrokerManager>>,
) -> Result<Vec<TradeDto>, IpcError> {
    let (_name, account) = account(&broker).await?;
    Ok(account
        .trades()
        .await
        .map_err(IpcError::from)?
        .iter()
        .map(Into::into)
        .collect())
}

#[tauri::command]
pub async fn broker_funds(broker: State<'_, Arc<BrokerManager>>) -> Result<FundsDto, IpcError> {
    let (_name, account) = account(&broker).await?;
    Ok(account.funds().await.map_err(IpcError::from)?.into())
}
