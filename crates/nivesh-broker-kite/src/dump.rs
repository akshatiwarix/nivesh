//! The Kite instrument dump: the whole market in one CSV, fetched daily and
//! reduced to cash-equity rows in the `broker_tokens` map.
//!
//! The dump carries every segment (F&O, currency, commodities, indices); a
//! cash-only Nivesh keeps `exchange` NSE/BSE rows with `instrument_type EQ`.
//! The fetch is all-or-nothing: an unreadable or empty dump changes nothing.

use nivesh_broker::BrokerError;
use nivesh_store::{BrokerTokenRow, Store};
use std::sync::Arc;

use crate::rest::{KiteRest, RateClass};

/// The broker name under which Kite's tokens are stored.
pub const KITE: &str = "kite";

/// Fetches the dump and replaces Kite's token map, wholesale. Returns the
/// number of rows stored.
pub async fn refresh(store: &Arc<Store>, rest: &KiteRest) -> Result<usize, BrokerError> {
    let text = rest.get_text(RateClass::General, "/instruments").await?;
    let rows = parse_instrument_csv(&text)?;
    store
        .replace_broker_tokens(KITE, &rows)
        .map_err(|e| BrokerError::Upstream(format!("storing the token map: {e}")))?;
    Ok(rows.len())
}

/// Parses the instrument dump CSV. The header names the columns, so the
/// order can change without notice; rows that do not name an exchange, a
/// symbol and a numeric token are skipped.
pub fn parse_instrument_csv(text: &str) -> Result<Vec<BrokerTokenRow>, BrokerError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(text.as_bytes());
    let headers = reader
        .headers()
        .map_err(|e| BrokerError::Upstream(format!("instrument dump header: {e}")))?
        .clone();
    let column = |name: &str| headers.iter().position(|h| h.eq_ignore_ascii_case(name));
    let Some(token_col) = column("instrument_token") else {
        return Err(BrokerError::Upstream(
            "instrument dump has no instrument_token column".into(),
        ));
    };
    let symbol_col = column("tradingsymbol");
    let exchange_col = column("exchange");
    let kind_col = column("instrument_type");
    let tick_col = column("tick_size");
    let lot_col = column("lot_size");

    let mut rows = Vec::new();
    for record in reader.records() {
        let record =
            record.map_err(|e| BrokerError::Upstream(format!("instrument dump row: {e}")))?;
        if record.len() <= token_col {
            continue;
        }
        let Some(token) = record[token_col].trim().parse::<i64>().ok() else {
            continue;
        };
        let exchange = symbol_col
            .and_then(|c| record.get(c))
            .and_then(|_| exchange_col.and_then(|c| record.get(c)))
            .unwrap_or_default()
            .trim()
            .to_ascii_uppercase();
        let symbol = symbol_col
            .and_then(|c| record.get(c))
            .unwrap_or_default()
            .trim()
            .to_ascii_uppercase();
        if symbol.is_empty() || !matches!(exchange.as_str(), "NSE" | "BSE") {
            continue;
        }
        let kind = kind_col
            .and_then(|c| record.get(c))
            .unwrap_or("EQ")
            .trim()
            .to_ascii_uppercase();
        if kind != "EQ" {
            continue;
        }
        let tick_size = tick_col
            .and_then(|c| record.get(c))
            .and_then(|v| v.trim().parse::<f64>().ok())
            .unwrap_or(0.05);
        let lot_size = lot_col
            .and_then(|c| record.get(c))
            .and_then(|v| v.trim().parse::<i64>().ok())
            .unwrap_or(1);
        rows.push(BrokerTokenRow {
            instrument_token: token,
            exchange,
            symbol,
            kind,
            tick_size,
            lot_size,
        });
    }
    if rows.is_empty() {
        return Err(BrokerError::Upstream(
            "instrument dump has no cash-equity rows".into(),
        ));
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const DUMP: &str = include_str!("../tests/fixtures/kite/instruments.csv");

    #[test]
    fn keeps_cash_equity_rows_only() {
        let rows = parse_instrument_csv(DUMP).unwrap();
        assert_eq!(rows.len(), 3, "RELIANCE NSE, TCS NSE, RELIANCE BSE");
        let reliance = &rows[0];
        assert_eq!(reliance.instrument_token, 738561);
        assert_eq!(reliance.exchange, "NSE");
        assert_eq!(reliance.symbol, "RELIANCE");
        assert_eq!(reliance.kind, "EQ");
        assert_eq!(reliance.tick_size, 0.05);
        assert_eq!(reliance.lot_size, 1);
    }

    #[test]
    fn a_headerless_or_empty_dump_is_an_error() {
        assert!(parse_instrument_csv("").is_err());
        assert!(parse_instrument_csv("instrument_token,tradingsymbol,exchange\n").is_err());
    }

    #[tokio::test]
    async fn refresh_replaces_the_map_and_lookups_work_both_ways() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/instruments"))
            .respond_with(ResponseTemplate::new(200).set_body_string(DUMP))
            .mount(&server)
            .await;
        let store = Arc::new(nivesh_store::Store::open_in_memory().unwrap());
        let rest = KiteRest::new(Some(server.uri()), "k".into()).unwrap();
        rest.set_token(secrecy::SecretString::from("t"));
        let stored = refresh(&store, &rest).await.unwrap();
        assert_eq!(stored, 3);

        // Forward: instrument → token (NSE wins when both list it).
        assert_eq!(
            store.token_for_instrument(KITE, "NSE", "RELIANCE").unwrap(),
            Some(738561)
        );
        // Backward: token → instrument.
        assert_eq!(
            store.instrument_for_token(KITE, 2953217).unwrap(),
            Some(("NSE".into(), "TCS".into()))
        );
        assert_eq!(store.instrument_for_token(KITE, 999999).unwrap(), None);
        assert_eq!(store.broker_token_count(KITE).unwrap(), 3);

        // A broken dump leaves the previous map intact.
        let bad = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not,a,csv"))
            .mount(&bad)
            .await;
        let rest_bad = KiteRest::new(Some(bad.uri()), "k".into()).unwrap();
        rest_bad.set_token(secrecy::SecretString::from("t"));
        assert!(refresh(&store, &rest_bad).await.is_err());
        assert_eq!(store.broker_token_count(KITE).unwrap(), 3);

        // Clearing works.
        store.clear_broker_tokens(KITE).unwrap();
        assert_eq!(store.broker_token_count(KITE).unwrap(), 0);
    }
}
