//! Market-data commands: search, quotes, indices and the quote subscription.
//! Thin: parse arguments, call a service, map the error.

use std::sync::Arc;

use nivesh_core::{Exchange, InstrumentId};
use nivesh_data::SubscriptionId;
use tauri::State;
use tauri::ipc::Channel;
use tokio::sync::mpsc;

use crate::ipc::IpcError;
use crate::ipc::dto::{
    HubUpdateDto, IndexSnapshotDto, InstrumentDto, ProviderStatusDto, SourcedIndexQuoteDto,
    SourcedQuoteDto, TopicDto, parse_index,
};
use crate::services::Services;

const DEFAULT_SEARCH_LIMIT: u32 = 10;
const MAX_SEARCH_LIMIT: u32 = 50;
const MAX_QUERY_LEN: usize = 64;
const MAX_TOPICS: usize = 200;

type Svc<'a> = State<'a, Arc<Services>>;

pub(crate) fn instrument_id(exchange: &str, symbol: &str) -> Result<InstrumentId, IpcError> {
    let exchange: Exchange = exchange
        .parse()
        .map_err(|e: nivesh_core::InstrumentIdError| IpcError::invalid_argument(e.to_string()))?;
    InstrumentId::new(exchange, symbol).map_err(|e| IpcError::invalid_argument(e.to_string()))
}

/// Ranked matches from the local instrument list. Never touches the network.
#[tauri::command]
pub async fn search_instruments(
    query: String,
    limit: Option<u32>,
    services: Svc<'_>,
) -> Result<Vec<InstrumentDto>, IpcError> {
    if query.len() > MAX_QUERY_LEN {
        return Err(IpcError::invalid_argument("search text is too long"));
    }
    let limit = limit
        .unwrap_or(DEFAULT_SEARCH_LIMIT)
        .clamp(1, MAX_SEARCH_LIMIT) as usize;
    Ok(services
        .instruments
        .search(&query, limit)
        .iter()
        .map(Into::into)
        .collect())
}

#[tauri::command]
pub async fn get_quote(
    exchange: String,
    symbol: String,
    services: Svc<'_>,
) -> Result<SourcedQuoteDto, IpcError> {
    let id = instrument_id(&exchange, &symbol)?;
    Ok((&services.market.quote(&id).await?).into())
}

#[tauri::command]
pub async fn index_snapshot(
    index: String,
    services: Svc<'_>,
) -> Result<IndexSnapshotDto, IpcError> {
    let index = parse_index(&index)?;
    Ok((&services.market.index_snapshot(index).await?).into())
}

/// Levels of every catalogued index, for the MKT dashboard.
#[tauri::command]
pub async fn all_indices(services: Svc<'_>) -> Result<Vec<SourcedIndexQuoteDto>, IpcError> {
    Ok(services
        .market
        .all_indices()
        .await?
        .iter()
        .map(Into::into)
        .collect())
}

/// Starts streaming updates for `topics` over `on_update`. Returns an id for
/// `unsubscribe`. The subscription also ends when the channel closes.
#[tauri::command]
pub async fn subscribe_quotes(
    topics: Vec<TopicDto>,
    on_update: Channel<HubUpdateDto>,
    services: Svc<'_>,
) -> Result<u64, IpcError> {
    if topics.len() > MAX_TOPICS {
        return Err(IpcError::invalid_argument(format!(
            "at most {MAX_TOPICS} topics per subscription"
        )));
    }
    let topics = topics
        .into_iter()
        .map(TopicDto::into_topic)
        .collect::<Result<Vec<_>, _>>()?;

    let (tx, mut rx) = mpsc::unbounded_channel();
    let id = services.hub.subscribe(topics, tx);
    let hub = services.hub.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(update) = rx.recv().await {
            if on_update.send(HubUpdateDto::from(&update)).is_err() {
                // The webview is gone: stop polling for it.
                hub.unsubscribe(id);
                break;
            }
        }
    });
    Ok(id.0)
}

#[tauri::command]
pub async fn unsubscribe(id: u64, services: Svc<'_>) -> Result<(), IpcError> {
    services.hub.unsubscribe(SubscriptionId(id));
    Ok(())
}

/// The UI reports whether the window is visible; polling pauses while hidden.
#[tauri::command]
pub async fn set_window_visible(visible: bool, services: Svc<'_>) -> Result<(), IpcError> {
    services.hub.set_visible(visible);
    Ok(())
}

#[tauri::command]
pub async fn provider_status(services: Svc<'_>) -> Result<Vec<ProviderStatusDto>, IpcError> {
    Ok(services
        .market
        .provider_status()
        .iter()
        .map(Into::into)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::ErrorCode;

    #[test]
    fn instrument_ids_validate_exchange_and_symbol() {
        assert_eq!(
            instrument_id("nse", "reliance").unwrap().to_string(),
            "NSE:RELIANCE"
        );
        assert_eq!(
            instrument_id("BSE", "500325").unwrap().exchange(),
            Exchange::Bse
        );
        for (exchange, symbol) in [
            ("LSE", "X"),
            ("NSE", ""),
            ("NSE", "bad symbol"),
            ("NSE", &"A".repeat(40)),
        ] {
            let err = instrument_id(exchange, symbol).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidArgument, "{exchange}:{symbol}");
        }
    }
}
