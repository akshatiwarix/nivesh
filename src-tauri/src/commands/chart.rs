//! Chart commands: candle history and the forming bar. Thin, like the rest.

use std::sync::Arc;

use nivesh_core::{Interval, Sourced};
use nivesh_data::forming::FormingBar;
use nivesh_data::{HistoryRange, HistoryRequest, HubUpdate, Topic};
use tauri::State;
use tauri::ipc::Channel;
use tokio::sync::mpsc;

use crate::commands::data::instrument_id;
use crate::ipc::IpcError;
use crate::ipc::chart::{BarDto, HistoryDto};
use crate::ipc::dto::ProvenanceDto;
use crate::services::Services;

type Svc<'a> = State<'a, Arc<Services>>;

/// Parses the interval and range arguments. A missing range means "as much as
/// the public sources serve for that interval".
pub fn parse_request(
    exchange: &str,
    symbol: &str,
    interval: &str,
    range: Option<&str>,
) -> Result<HistoryRequest, IpcError> {
    let interval: Interval = interval.parse().map_err(IpcError::invalid_argument)?;
    let range = match range {
        Some(r) => r.parse().map_err(IpcError::invalid_argument)?,
        None => HistoryRange::default_for(interval),
    };
    Ok(HistoryRequest {
        id: instrument_id(exchange, symbol)?,
        interval,
        range,
    })
}

/// Candles for a chart, as columns. Served from cache within its TTL; a
/// provider that can't cover the range gives an error that says why.
#[tauri::command]
pub async fn get_history(
    exchange: String,
    symbol: String,
    interval: String,
    range: Option<String>,
    services: Svc<'_>,
) -> Result<HistoryDto, IpcError> {
    let req = parse_request(&exchange, &symbol, &interval, range.as_deref())?;
    let series = services.market.history(&req).await?;
    Ok(HistoryDto::new(&series, req.range.code()))
}

/// Streams the forming bar of a chart over `on_bar` until `unsubscribe(id)`
/// (the same id space as quote subscriptions) or the channel closes. The bar is
/// built in Rust from the quote stream; the view only draws it.
#[tauri::command]
pub async fn subscribe_forming_bar(
    exchange: String,
    symbol: String,
    interval: String,
    range: Option<String>,
    on_bar: Channel<BarDto>,
    services: Svc<'_>,
) -> Result<u64, IpcError> {
    let req = parse_request(&exchange, &symbol, &interval, range.as_deref())?;
    // Normally a cache hit: the chart has just loaded this series.
    let last = services
        .market
        .history(&req)
        .await?
        .data
        .candles
        .last()
        .copied();
    let mut forming = FormingBar::new(req.interval, last);

    let (tx, mut rx) = mpsc::unbounded_channel();
    let id = services
        .hub
        .subscribe(vec![Topic::Quote(req.id.clone())], tx);
    let hub = services.hub.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(update) = rx.recv().await {
            let HubUpdate::Pinned(Sourced { data, provenance }) = update else {
                continue;
            };
            if let Some(bar) = forming.apply(data.ltp, provenance.as_of.timestamp_millis())
                && on_bar
                    .send(BarDto::new(&bar, ProvenanceDto::from(&provenance)))
                    .is_err()
            {
                // The webview is gone: stop polling for it.
                hub.unsubscribe(id);
                return;
            }
        }
    });
    Ok(id.0)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::ErrorCode;

    #[test]
    fn requests_parse_and_default_the_range() {
        let r = parse_request("nse", "reliance", "5M", None).unwrap();
        assert_eq!(r.interval, Interval::M5);
        assert_eq!(r.range, HistoryRange::M1);
        let r = parse_request("NSE", "TCS", "1d", Some("5Y")).unwrap();
        assert_eq!(r.range, HistoryRange::Y5);
        for (interval, range) in [("7m", None), ("5m", Some("2y"))] {
            let err = parse_request("NSE", "TCS", interval, range).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidArgument);
        }
    }
}
