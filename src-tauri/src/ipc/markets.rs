//! Wire types for the market views: movers, breadth, sectors and news.

use nivesh_core::Sourced;
use nivesh_data::Movers;
use serde::Serialize;
use ts_rs::TS;

use crate::ipc::dto::{ProvenanceDto, QuoteDto};

/// Top movers of one index universe, from one source.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MoversDto {
    /// Index code, e.g. `NIFTY50`.
    pub index: String,
    pub gainers: Vec<QuoteDto>,
    pub losers: Vec<QuoteDto>,
    /// By volume. Empty when the source reports no volume per stock.
    pub most_active: Vec<QuoteDto>,
    pub provenance: ProvenanceDto,
}

impl MoversDto {
    pub fn new(index: &str, movers: &Sourced<Movers>) -> Self {
        let rows =
            |list: &[nivesh_data::Mover]| list.iter().map(|m| QuoteDto::from(&m.quote)).collect();
        Self {
            index: index.to_owned(),
            gainers: rows(&movers.data.gainers),
            losers: rows(&movers.data.losers),
            most_active: rows(&movers.data.most_active),
            provenance: (&movers.provenance).into(),
        }
    }
}
