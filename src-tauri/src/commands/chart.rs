//! Chart commands: candle history and the forming bar. Thin, like the rest.

use std::sync::Arc;

use nivesh_core::Candle;
use nivesh_core::{Interval, Sourced};
use nivesh_data::forming::FormingBar;
use nivesh_data::{HistoryRange, HistoryRequest, HubUpdate, Topic};
use nivesh_indicators::{IndicatorSpec, Ohlcv, compute};
use nivesh_store::Store;
use tauri::State;
use tauri::ipc::Channel;
use tokio::sync::mpsc;

use crate::commands::data::instrument_id;
use crate::ipc::IpcError;
use crate::ipc::chart::{
    BarDto, HistoryDto, IndicatorDto, IndicatorPreviewDto, IndicatorPreviewLinesDto, IndicatorsDto,
};
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
/// Most indicators one chart may carry.
const MAX_INDICATORS: usize = 8;

fn parse_specs(specs: &[String]) -> Result<Vec<IndicatorSpec>, IpcError> {
    if specs.len() > MAX_INDICATORS {
        return Err(IpcError::invalid_argument(format!(
            "at most {MAX_INDICATORS} indicators per chart"
        )));
    }
    specs
        .iter()
        .map(|s| {
            s.parse().map_err(|e: nivesh_indicators::SpecError| {
                IpcError::invalid_argument(e.to_string())
            })
        })
        .collect()
}

/// Indicator lines for a chart's series, computed in Rust from the cached
/// candles. The result carries the bar times it was computed for.
#[tauri::command]
pub async fn compute_indicators(
    exchange: String,
    symbol: String,
    interval: String,
    range: Option<String>,
    specs: Vec<String>,
    services: Svc<'_>,
) -> Result<IndicatorsDto, IpcError> {
    let specs = parse_specs(&specs)?;
    let req = parse_request(&exchange, &symbol, &interval, range.as_deref())?;
    let series = services.market.history(&req).await?;
    let candles = &series.data.candles;
    let t: Vec<i64> = candles.iter().map(|c| c.t).collect();
    let column =
        |f: fn(&nivesh_core::Candle) -> f64| -> Vec<f64> { candles.iter().map(f).collect() };
    let (h, l, c, v) = (
        column(|c| c.h),
        column(|c| c.l),
        column(|c| c.c),
        column(|c| c.v),
    );
    let data = Ohlcv {
        t: &t,
        h: &h,
        l: &l,
        c: &c,
        v: &v,
    };
    let indicators = specs
        .iter()
        .map(|spec| IndicatorDto::from(&compute(spec, &data)))
        .collect();
    Ok(IndicatorsDto { t, indicators })
}

/// Folds the forming bar into the loaded candles: it replaces the last candle
/// while it is the same bar still forming, and is appended once it has rolled
/// into a new one. An older bar is stale; the loaded series wins.
fn fold_forming_bar(candles: &mut Vec<Candle>, bar: Candle) {
    match candles.last_mut() {
        Some(last) if last.t == bar.t => *last = bar,
        Some(last) if last.t < bar.t => candles.push(bar),
        None => candles.push(bar),
        // Older than the loaded series: the series wins, nothing to do.
        _ => {}
    }
}

/// The last values of a chart's indicators with the forming bar folded in:
/// the candles `compute_indicators` used, with `bar` replacing the last
/// candle while it is still forming and appended once it has rolled into a
/// new one. Lines match `compute_indicators`' order per id; the chart only
/// moves its newest point.
#[tauri::command]
pub async fn preview_indicators(
    exchange: String,
    symbol: String,
    interval: String,
    range: Option<String>,
    specs: Vec<String>,
    bar: BarDto,
    services: Svc<'_>,
) -> Result<IndicatorPreviewDto, IpcError> {
    let specs = parse_specs(&specs)?;
    let req = parse_request(&exchange, &symbol, &interval, range.as_deref())?;
    let series = services.market.history(&req).await?;
    let mut candles = series.data.candles.clone();
    fold_forming_bar(&mut candles, bar.candle());
    let t: Vec<i64> = candles.iter().map(|c| c.t).collect();
    let column =
        |f: fn(&nivesh_core::Candle) -> f64| -> Vec<f64> { candles.iter().map(f).collect() };
    let (h, l, c, v) = (
        column(|c| c.h),
        column(|c| c.l),
        column(|c| c.c),
        column(|c| c.v),
    );
    let data = Ohlcv {
        t: &t,
        h: &h,
        l: &l,
        c: &c,
        v: &v,
    };
    let indicators = specs
        .iter()
        .map(|spec| {
            let result = compute(spec, &data);
            IndicatorPreviewLinesDto {
                id: result.id.clone(),
                lines: result
                    .lines
                    .iter()
                    .map(|line| line.values.last().copied().filter(|v| v.is_finite()))
                    .collect(),
            }
        })
        .collect();
    let t = t.last().copied().unwrap_or(bar.t);
    Ok(IndicatorPreviewDto { t, indicators })
}

fn indicators_key(exchange: &str, symbol: &str) -> Result<String, IpcError> {
    let id = instrument_id(exchange, symbol)?;
    Ok(format!("chart.indicators.{id}"))
}

/// The indicators saved for this symbol, in the order they were added. Entries
/// that no longer parse are dropped.
#[tauri::command]
pub async fn chart_indicators(
    exchange: String,
    symbol: String,
    store: State<'_, Arc<Store>>,
) -> Result<Vec<String>, IpcError> {
    let key = indicators_key(&exchange, &symbol)?;
    let saved: Vec<String> = store
        .setting(&key)?
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    Ok(saved
        .iter()
        .filter_map(|s| s.parse::<IndicatorSpec>().ok())
        .map(|s| s.id())
        .take(MAX_INDICATORS)
        .collect())
}

#[tauri::command]
pub async fn save_chart_indicators(
    exchange: String,
    symbol: String,
    specs: Vec<String>,
    store: State<'_, Arc<Store>>,
) -> Result<(), IpcError> {
    let key = indicators_key(&exchange, &symbol)?;
    let canonical: Vec<String> = parse_specs(&specs)?.iter().map(IndicatorSpec::id).collect();
    let json = serde_json::to_string(&canonical)
        .map_err(|e| IpcError::internal(format!("cannot encode indicators: {e}")))?;
    Ok(store.set_setting(&key, &json)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::ErrorCode;
    use nivesh_core::Candle;

    fn candle(t: i64, c: f64) -> Candle {
        Candle {
            t,
            o: c,
            h: c,
            l: c,
            c,
            v: 1.0,
        }
    }

    #[test]
    fn the_forming_bar_replaces_its_bar_and_appends_the_next() {
        let mut candles = vec![candle(1000, 10.0), candle(2000, 11.0)];
        fold_forming_bar(&mut candles, candle(2000, 12.5));
        assert_eq!(candles[1].c, 12.5);
        assert_eq!(candles.len(), 2, "still the same bar");
        fold_forming_bar(&mut candles, candle(3000, 13.0));
        assert_eq!(candles.len(), 3);
        assert_eq!(candles[2].c, 13.0);
        // A stale bar changes nothing.
        fold_forming_bar(&mut candles, candle(500, 9.0));
        assert_eq!(candles.len(), 3);
    }

    #[test]
    fn indicator_specs_are_validated_and_bounded() {
        let ok = parse_specs(&["sma 50".into(), "RSI".into()]).unwrap();
        assert_eq!(
            ok.iter().map(IndicatorSpec::id).collect::<Vec<_>>(),
            ["SMA 50", "RSI 14"]
        );
        assert_eq!(
            parse_specs(&["FOO".into()]).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        let many = vec!["SMA 5".to_owned(); MAX_INDICATORS + 1];
        assert!(parse_specs(&many).is_err());
    }

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
