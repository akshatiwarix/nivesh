//! Market-view commands: movers, breadth, sectors and news. Thin, like the rest.

use std::sync::Arc;

use nivesh_data::{DmaUniverse, MoversRequest, Progress};
use tauri::State;
use tauri::ipc::Channel;

use crate::ipc::IpcError;
use crate::ipc::dto::parse_index;
use crate::ipc::markets::{BreadthDto, DmaBreadthDto, MoversDto, ProgressDto, SectorsDto};
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
    let movers = services
        .market
        .movers(&MoversRequest { index: id, limit })
        .await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::ErrorCode;

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
