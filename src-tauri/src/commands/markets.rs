//! Market-view commands: movers, breadth, sectors and news. Thin, like the rest.

use std::sync::Arc;

use nivesh_data::news::is_openable;
use nivesh_data::{DmaUniverse, MoversRequest, NewsRequest, Progress};
use tauri::State;
use tauri::ipc::Channel;

use crate::ipc::dto::parse_index;
use crate::ipc::markets::{BreadthDto, DmaBreadthDto, MoversDto, NewsDto, ProgressDto, SectorsDto};
use crate::ipc::{ErrorCode, IpcError};
use crate::services::Services;

type Svc<'a> = State<'a, Arc<Services>>;

const DEFAULT_MOVERS: u32 = 10;
const MAX_MOVERS: u32 = 25;

/// Gainers, losers and most active stocks of an index universe.
#[tauri::command]
pub async fn get_movers(
    index: String,
    limit: Option<u32>,
    services: Svc<'_>,
) -> Result<MoversDto, IpcError> {
    let id = parse_index(&index)?;
    let limit = limit.unwrap_or(DEFAULT_MOVERS).clamp(1, MAX_MOVERS) as usize;
    let mut movers = services
        .market
        .movers(&MoversRequest { index: id, limit })
        .await?;
    // Some sources list symbols only; the names come from the local list.
    for m in movers
        .data
        .gainers
        .iter_mut()
        .chain(movers.data.losers.iter_mut())
        .chain(movers.data.most_active.iter_mut())
    {
        if m.quote.name.is_none() {
            m.quote.name = services.instruments.get(&m.quote.id).map(|i| i.name);
        }
    }
    Ok(MoversDto::new(id.code(), &movers))
}

/// Advances, declines and unchanged among an index's stocks.
#[tauri::command]
pub async fn get_breadth(index: String, services: Svc<'_>) -> Result<BreadthDto, IpcError> {
    let id = parse_index(&index)?;
    Ok((&services.market.index_breadth(id).await?).into())
}

fn parse_universe(text: &str) -> Result<DmaUniverse, IpcError> {
    match text.trim().to_ascii_uppercase().as_str() {
        "NIFTY50" => Ok(DmaUniverse::Nifty50),
        "NIFTY100" => Ok(DmaUniverse::Nifty100),
        other => Err(IpcError::invalid_argument(format!(
            "moving-average breadth covers NIFTY50 and NIFTY100, not `{other}`"
        ))),
    }
}

/// Share of NIFTY 50 or NIFTY 100 above their 20, 50 and 200-day averages.
/// A cold run fetches a year of daily candles per stock (a minute or so at the
/// shared rate limit), reporting progress on `on_progress`.
#[tauri::command]
pub async fn get_dma_breadth(
    universe: String,
    on_progress: Channel<ProgressDto>,
    services: Svc<'_>,
) -> Result<DmaBreadthDto, IpcError> {
    let universe = parse_universe(&universe)?;
    let progress: Progress = Arc::new(move |done, total| {
        // The view may have closed; the sweep still finishes into the cache.
        let _ = on_progress.send(ProgressDto {
            done: u32::try_from(done).unwrap_or(u32::MAX),
            total: u32::try_from(total).unwrap_or(u32::MAX),
        });
    });
    Ok((&services.market.dma_breadth(universe, progress).await?).into())
}

/// Returns of every index over 1D, 1W, 1M and year to date.
#[tauri::command]
pub async fn get_sectors(services: Svc<'_>) -> Result<SectorsDto, IpcError> {
    Ok((&services.market.sector_performance().await?).into())
}

const DEFAULT_NEWS: u32 = 60;
const MAX_NEWS: u32 = 200;
const MAX_NEWS_QUERY: usize = 32;

/// Recent market headlines, newest first. `query` is a symbol or a word to
/// look for in the headline (`RELIANCE`).
#[tauri::command]
pub async fn get_news(
    query: Option<String>,
    limit: Option<u32>,
    services: Svc<'_>,
) -> Result<NewsDto, IpcError> {
    let query = query.map(|q| q.trim().to_owned()).filter(|q| !q.is_empty());
    if query.as_ref().is_some_and(|q| q.len() > MAX_NEWS_QUERY) {
        return Err(IpcError::invalid_argument("news search text is too long"));
    }
    let limit = limit.unwrap_or(DEFAULT_NEWS).clamp(1, MAX_NEWS) as usize;
    let news = services
        .market
        .news(&NewsRequest {
            query: query.clone(),
            limit,
        })
        .await?;
    Ok(NewsDto::new(query, &news))
}

/// Opens a headline's link in the system browser. This is the only way the app
/// opens a URL, and it is deliberately narrow: the link must be http or https
/// *and* be one the news feed itself supplied, so the webview can't use it to
/// open anything else. The page is never loaded into the app.
#[tauri::command]
pub async fn open_news_link(
    url: String,
    app: tauri::AppHandle,
    services: Svc<'_>,
) -> Result<(), IpcError> {
    if !is_openable(&url) {
        return Err(IpcError::invalid_argument(
            "only http and https links can be opened",
        ));
    }
    let known = services
        .market
        .news(&NewsRequest {
            query: None,
            limit: MAX_NEWS as usize,
        })
        .await?
        .data
        .iter()
        .any(|item| item.url == url);
    if !known {
        return Err(IpcError::invalid_argument(
            "that link is not from the news feed",
        ));
    }
    tauri_plugin_opener::OpenerExt::opener(&app)
        .open_url(&url, None::<&str>)
        .map_err(|e| {
            IpcError::new(
                ErrorCode::Unavailable,
                format!("could not open the browser: {e}"),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_two_covered_universes_are_accepted() {
        assert_eq!(parse_universe("nifty50").unwrap(), DmaUniverse::Nifty50);
        assert_eq!(parse_universe(" NIFTY100 ").unwrap(), DmaUniverse::Nifty100);
        assert_eq!(
            parse_universe("NIFTYIT").unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }
}
