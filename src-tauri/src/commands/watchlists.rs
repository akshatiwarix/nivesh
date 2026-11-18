//! Watchlist commands. Every mutation returns the full, updated state so the
//! UI never has to guess what changed.

use std::sync::Arc;

use nivesh_core::{Exchange, InstrumentId};
use nivesh_store::{Store, WatchItem, Watchlist};
use serde::Serialize;
use tauri::State;
use ts_rs::TS;

use crate::ipc::IpcError;
use crate::services::Services;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WatchItemDto {
    pub exchange: String,
    pub symbol: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WatchlistDto {
    pub id: i64,
    pub name: String,
    pub items: Vec<WatchItemDto>,
}

/// All lists in order, and which one `WL ADD` writes to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WatchlistsDto {
    pub lists: Vec<WatchlistDto>,
    pub active: i64,
}

impl From<WatchItem> for WatchItemDto {
    fn from(i: WatchItem) -> Self {
        Self {
            exchange: i.exchange,
            symbol: i.symbol,
        }
    }
}

impl From<Watchlist> for WatchlistDto {
    fn from(l: Watchlist) -> Self {
        Self {
            id: l.id,
            name: l.name,
            items: l.items.into_iter().map(Into::into).collect(),
        }
    }
}

fn snapshot(store: &Store) -> Result<WatchlistsDto, IpcError> {
    Ok(WatchlistsDto {
        lists: store.watchlists()?.into_iter().map(Into::into).collect(),
        active: store.active_watchlist()?,
    })
}

fn target_list(store: &Store, list_id: Option<i64>) -> Result<i64, IpcError> {
    Ok(match list_id {
        Some(id) => id,
        None => store.active_watchlist()?,
    })
}

fn parse_id(exchange: &str, symbol: &str) -> Result<InstrumentId, IpcError> {
    let exchange: Exchange = exchange
        .parse()
        .map_err(|e: nivesh_core::InstrumentIdError| IpcError::invalid_argument(e.to_string()))?;
    InstrumentId::new(exchange, symbol).map_err(|e| IpcError::invalid_argument(e.to_string()))
}

/// An NSE symbol must be in the instrument list (a typo would otherwise sit in
/// the list forever with no quote). BSE has no list, so any valid code passes.
fn check_listed(id: &InstrumentId, services: &Services) -> Result<(), IpcError> {
    let listed = id.exchange() != Exchange::Nse
        || services.instruments.is_empty()
        || services.instruments.get(id).is_some();
    if listed {
        Ok(())
    } else {
        Err(IpcError::invalid_argument(format!(
            "{} is not a listed NSE symbol",
            id.symbol()
        )))
    }
}

type Db<'a> = State<'a, Arc<Store>>;
type Svc<'a> = State<'a, Arc<Services>>;

#[tauri::command]
pub async fn watchlists(store: Db<'_>) -> Result<WatchlistsDto, IpcError> {
    snapshot(&store)
}

#[tauri::command]
pub async fn watchlist_create(name: String, store: Db<'_>) -> Result<WatchlistsDto, IpcError> {
    let id = store.create_watchlist(&name)?;
    store.set_active_watchlist(id)?;
    snapshot(&store)
}

#[tauri::command]
pub async fn watchlist_rename(
    id: i64,
    name: String,
    store: Db<'_>,
) -> Result<WatchlistsDto, IpcError> {
    store.rename_watchlist(id, &name)?;
    snapshot(&store)
}

#[tauri::command]
pub async fn watchlist_delete(id: i64, store: Db<'_>) -> Result<WatchlistsDto, IpcError> {
    store.delete_watchlist(id)?;
    snapshot(&store)
}

#[tauri::command]
pub async fn watchlist_activate(id: i64, store: Db<'_>) -> Result<WatchlistsDto, IpcError> {
    store.set_active_watchlist(id)?;
    snapshot(&store)
}

#[tauri::command]
pub async fn watchlist_add(
    exchange: String,
    symbol: String,
    list_id: Option<i64>,
    store: Db<'_>,
    services: Svc<'_>,
) -> Result<WatchlistsDto, IpcError> {
    let id = parse_id(&exchange, &symbol)?;
    check_listed(&id, &services)?;
    let list = target_list(&store, list_id)?;
    store.add_watch_item(list, id.exchange().code(), id.symbol())?;
    snapshot(&store)
}

#[tauri::command]
pub async fn watchlist_remove(
    exchange: String,
    symbol: String,
    list_id: Option<i64>,
    store: Db<'_>,
) -> Result<WatchlistsDto, IpcError> {
    let id = parse_id(&exchange, &symbol)?;
    let list = target_list(&store, list_id)?;
    store.remove_watch_item(list, id.exchange().code(), id.symbol())?;
    snapshot(&store)
}

#[tauri::command]
pub async fn watchlist_move(
    exchange: String,
    symbol: String,
    delta: i32,
    list_id: Option<i64>,
    store: Db<'_>,
) -> Result<WatchlistsDto, IpcError> {
    let id = parse_id(&exchange, &symbol)?;
    let list = target_list(&store, list_id)?;
    store.move_watch_item(
        list,
        id.exchange().code(),
        id.symbol(),
        delta.clamp(-1000, 1000),
    )?;
    snapshot(&store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::ErrorCode;

    #[test]
    fn snapshot_reports_lists_items_and_the_active_list() {
        let store = Store::open_in_memory().unwrap();
        let first = store.active_watchlist().unwrap();
        store.add_watch_item(first, "NSE", "TCS").unwrap();
        let second = store.create_watchlist("banks").unwrap();
        store.set_active_watchlist(second).unwrap();
        let snap = snapshot(&store).unwrap();
        assert_eq!(snap.active, second);
        assert_eq!(snap.lists.len(), 2);
        assert_eq!(
            snap.lists[0].items,
            [WatchItemDto {
                exchange: "NSE".into(),
                symbol: "TCS".into()
            }]
        );
        assert_eq!(snap.lists[1].name, "BANKS");
        let json = serde_json::to_value(&snap).unwrap();
        assert_eq!(json["lists"][0]["items"][0]["symbol"], "TCS");
    }

    #[test]
    fn target_list_defaults_to_the_active_one() {
        let store = Store::open_in_memory().unwrap();
        let active = store.active_watchlist().unwrap();
        assert_eq!(target_list(&store, None).unwrap(), active);
        assert_eq!(target_list(&store, Some(77)).unwrap(), 77);
    }

    #[test]
    fn symbols_are_validated() {
        assert_eq!(parse_id("nse", "tcs").unwrap().to_string(), "NSE:TCS");
        for (e, s) in [("XXX", "TCS"), ("NSE", ""), ("NSE", "no spaces")] {
            assert_eq!(parse_id(e, s).unwrap_err().code, ErrorCode::InvalidArgument);
        }
    }

    #[test]
    fn store_rule_violations_reach_the_ui_as_invalid_arguments() {
        let store = Store::open_in_memory().unwrap();
        let err: IpcError = store.create_watchlist("   ").unwrap_err().into();
        assert_eq!(err.code, ErrorCode::InvalidArgument);
        assert_eq!(err.message, "a watchlist needs a name");
        let err: IpcError = store.delete_watchlist(12345).unwrap_err().into();
        assert_eq!(err.code, ErrorCode::NotFound);
        let only = store.active_watchlist().unwrap();
        let err: IpcError = store.delete_watchlist(only).unwrap_err().into();
        assert_eq!(err.code, ErrorCode::InvalidArgument);
        assert!(!err.retryable);
    }
}
