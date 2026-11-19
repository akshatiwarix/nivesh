//! The Nivesh desktop app.
//!
//! This crate only wires things together: it builds the Tauri app, registers
//! plugins and commands, and owns process-level concerns such as logging and
//! the main window. Business logic lives in the `nivesh-*` library crates.

mod audit;
mod broker;
mod commands;
pub mod ipc;
mod logging;
mod paper;
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
        // Opens links in the system browser. No window is granted its commands:
        // the only caller is `open_news_link`, which vets the URL first.
        .plugin(tauri_plugin_opener::init())
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

            // Paper strategies saved as running resume from their saved
            // state. Paper only: the desk has no route to a real broker.
            let paper = Arc::new(paper::PaperManager::new(Arc::clone(&store)));
            {
                let paper = Arc::clone(&paper);
                let services = Arc::clone(&services);
                tauri::async_runtime::spawn(async move { paper.resume_all(&services).await });
            }
            app.manage(paper);
            app.manage(services);

            // Broker wiring: restore a stored session so LIVE data works
            // after a restart until the daily flush.
            let broker = Arc::new(broker::BrokerManager::new(Arc::clone(&store)));
            if let Err(err) = tauri::async_runtime::block_on(broker.restore()) {
                tracing::warn!(error = %err, "restoring the broker session failed");
            }
            app.manage(broker);

            window_state::restore(&window, &store);
            window_state::track(&window, store);
            window.show().context("showing main window")?;
            Ok(())
        })
        // Keep in sync with COMMANDS in build.rs.
        .invoke_handler(tauri::generate_handler![
            commands::app::app_info,
            commands::broker::broker_demo,
            commands::broker::broker_login,
            commands::broker::broker_logout,
            commands::broker::broker_funds,
            commands::broker::broker_holdings,
            commands::broker::broker_orders,
            commands::broker::broker_positions,
            commands::broker::broker_setup,
            commands::broker::broker_status,
            commands::broker::broker_trades,
            commands::risk::kill_engage,
            commands::risk::kill_reset,
            commands::risk::kill_status,
            commands::strategy::backtest_result,
            commands::strategy::backtest_strategy,
            commands::strategy::paper_start,
            commands::strategy::paper_states,
            commands::strategy::paper_stop,
            commands::strategy::strategies,
            commands::strategy::strategy_delete,
            commands::strategy::strategy_save,
            commands::chart::chart_indicators,
            commands::chart::compute_indicators,
            commands::chart::preview_indicators,
            commands::chart::get_history,
            commands::chart::save_chart_indicators,
            commands::chart::subscribe_forming_bar,
            commands::data::all_indices,
            commands::data::get_quote,
            commands::data::index_snapshot,
            commands::data::provider_status,
            commands::data::search_instruments,
            commands::data::set_window_visible,
            commands::data::subscribe_quotes,
            commands::data::unsubscribe,
            commands::market::market_status,
            commands::markets::get_breadth,
            commands::markets::get_dma_breadth,
            commands::markets::get_movers,
            commands::markets::get_news,
            commands::markets::open_news_link,
            commands::markets::get_sectors,
            commands::ui_state::ui_state,
            commands::ui_state::save_layout,
            commands::ui_state::save_command_history,
            commands::watchlists::watchlist_activate,
            commands::watchlists::watchlist_add,
            commands::watchlists::watchlist_create,
            commands::watchlists::watchlist_delete,
            commands::watchlists::watchlist_move,
            commands::watchlists::watchlist_remove,
            commands::watchlists::watchlist_rename,
            commands::watchlists::watchlists,
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
