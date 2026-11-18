//! The Nivesh desktop app.
//!
//! This crate only wires things together: it builds the Tauri app, registers
//! plugins and commands, and owns process-level concerns such as logging and
//! the main window. Business logic lives in the `nivesh-*` library crates.

mod commands;
pub mod ipc;
mod logging;
pub mod services;
mod window_state;

use std::sync::Arc;

use anyhow::Context as _;
use nivesh_store::Store;
use tauri::{AppHandle, Manager as _};

/// Label of the single main window (see `tauri.conf.json`).
pub const MAIN_WINDOW: &str = "main";

/// Builds and runs the app. Returns when the app exits.
pub fn run() -> anyhow::Result<()> {
    tauri::Builder::default()
        // Must be registered first: a second launch hands its arguments to the
        // running instance and exits before anything else starts. Two instances
        // would fight over the database and, later, could double-send orders.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            focus_main_window(app);
        }))
        .setup(|app| {
            let guard = logging::init(app.handle()).context("initialising logging")?;
            app.manage(guard);
            tracing::info!(
                version = env!("CARGO_PKG_VERSION"),
                debug = cfg!(debug_assertions),
                "nivesh starting"
            );

            let db_path = app
                .path()
                .app_data_dir()
                .context("resolving app data directory")?
                .join("nivesh.db");
            let store = Arc::new(Store::open(&db_path).context("opening database")?);
            app.manage(Arc::clone(&store));

            // The window is created hidden so it can be positioned before it
            // is first painted.
            let window = app
                .get_webview_window(MAIN_WINDOW)
                .context("main window missing from tauri.conf.json")?;
            let services = tauri::async_runtime::block_on(async { services::build(Arc::clone(&store), &services::Overrides::from_env()) })
                .map(Arc::new)
                .context("starting market data")?;
            services.spawn_background_tasks();
            services.spawn_event_bridges(app.handle());
            app.manage(services);

            window_state::restore(&window, &store);
            window_state::track(&window, store);
            window.show().context("showing main window")?;
            Ok(())
        })
        // Keep in sync with COMMANDS in build.rs.
        .invoke_handler(tauri::generate_handler![
            commands::app::app_info,
            commands::data::all_indices,
            commands::data::get_quote,
            commands::data::index_snapshot,
            commands::data::provider_status,
            commands::data::search_instruments,
            commands::data::set_window_visible,
            commands::data::subscribe_quotes,
            commands::data::unsubscribe,
            commands::market::market_status,
            commands::ui_state::ui_state,
            commands::ui_state::save_layout,
            commands::ui_state::save_command_history,
        ])
        .run(tauri::generate_context!())
        .context("running tauri app")
}

/// Brings the main window to the front, e.g. when a second instance starts.
fn focus_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };
    // Best effort: a failure here only means the window isn't raised.
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}
