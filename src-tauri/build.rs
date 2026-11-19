/// Commands the webview may invoke. tauri-build generates `allow-<command>`
/// and `deny-<command>` permissions for each, and `capabilities/main.json`
/// grants them explicitly. Keep in sync with `generate_handler!` in `lib.rs`.
const COMMANDS: &[&str] = &[
    "all_indices",
    "app_info",
    "broker_demo",
    "broker_login",
    "broker_logout",
    "broker_setup",
    "broker_funds",
    "broker_holdings",
    "broker_orders",
    "broker_positions",
    "broker_status",
    "broker_trades",
    "backtest_result",
    "backtest_strategy",
    "chart_indicators",
    "kill_engage",
    "kill_reset",
    "kill_status",
    "paper_start",
    "paper_states",
    "paper_stop",
    "strategies",
    "strategy_delete",
    "strategy_save",
    "compute_indicators",
    "preview_indicators",
    "get_breadth",
    "get_dma_breadth",
    "get_history",
    "get_movers",
    "get_news",
    "open_news_link",
    "get_sectors",
    "get_quote",
    "index_snapshot",
    "market_status",
    "provider_status",
    "save_chart_indicators",
    "search_instruments",
    "set_window_visible",
    "subscribe_forming_bar",
    "subscribe_quotes",
    "unsubscribe",
    "watchlist_activate",
    "watchlist_add",
    "watchlist_create",
    "watchlist_delete",
    "watchlist_move",
    "watchlist_remove",
    "watchlist_rename",
    "watchlists",
    "ui_state",
    "save_layout",
    "save_command_history",
];

fn main() {
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    if let Err(err) = tauri_build::try_build(attributes) {
        panic!("tauri-build failed: {err:#}");
    }
}
