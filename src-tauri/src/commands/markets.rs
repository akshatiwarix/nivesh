//! Market-view commands: movers, breadth, sectors and news. Thin, like the rest.

use std::sync::Arc;

use nivesh_data::MoversRequest;
use tauri::State;

use crate::ipc::IpcError;
use crate::ipc::dto::parse_index;
use crate::ipc::markets::MoversDto;
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
