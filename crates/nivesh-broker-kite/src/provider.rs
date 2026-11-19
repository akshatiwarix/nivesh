//! The broker-backed quote provider: when Kite is connected, the chain
//! routes quotes to it first and every value it serves is LIVE — real-time
//! broker data, unlike the public sources' DELAYED feeds.
//!
//! Quotes come from `/quote` (rate-limited to the docs' 1 req/s for the
//! quote class); history stays on the public sources until the historical
//! entitlement is wired. If the session dies, the provider surfaces
//! `Unauthorized`, the chain falls back to NSE/Yahoo, and the UI shows the
//! source change plus a re-login prompt.

use async_trait::async_trait;
use nivesh_broker::BrokerError;
use nivesh_core::{Exchange, FeedKind, InstrumentId, Provenance, ProviderId, Quote, Sourced};
use std::sync::Arc;

use crate::rest::{KiteRest, RateClass};
use nivesh_data::{Capabilities, DataError, DataProvider};

pub struct KiteQuoteProvider {
    rest: Arc<KiteRest>,
}

impl KiteQuoteProvider {
    pub fn new(rest: Arc<KiteRest>) -> Self {
        Self { rest }
    }
}

/// Maps a broker error onto the data taxonomy. `Unauthorized` is not
/// retryable for the chain: the broker session must be refreshed by a
/// login, so the chain just falls back to the next provider.
fn data_error(err: BrokerError) -> DataError {
    match err {
        BrokerError::Unauthorized => DataError::Unauthorized,
        BrokerError::RateLimited { retry_after } => DataError::RateLimited { retry_after },
        BrokerError::Timeout => DataError::Timeout,
        BrokerError::InvalidRequest(_) | BrokerError::NotFound(_) => DataError::NotFound,
        BrokerError::InvalidTransition { .. } | BrokerError::Unsupported => DataError::Unsupported,
        BrokerError::Upstream(text) => DataError::Upstream(text),
    }
}

/// `NSE:RELIANCE` — the wire form Kite's quotes use.
fn kite_key(id: &InstrumentId) -> String {
    format!("{}:{}", id.exchange().code(), id.symbol())
}

#[async_trait]
impl DataProvider for KiteQuoteProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Kite
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::QUOTE
    }

    fn feed_kind(&self) -> FeedKind {
        FeedKind::Live
    }

    async fn quotes(&self, ids: &[InstrumentId]) -> Result<Vec<Sourced<Quote>>, DataError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let query: Vec<(String, String)> = ids
            .iter()
            .map(|id| ("i".to_owned(), kite_key(id)))
            .collect();
        let data = self
            .rest
            .get(RateClass::Quote, "/quote", &query)
            .await
            .map_err(data_error)?;
        let quotes = data
            .as_object()
            .ok_or_else(|| DataError::Parse("quote payload is not an object".into()))?;
        let now = chrono::Utc::now();
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            let Some(row) = quotes.get(&kite_key(id)) else {
                continue;
            };
            let f64_at =
                |field: &str| -> Option<f64> { row.get(field).and_then(serde_json::Value::as_f64) };
            let ohlc = row.get("ohlc");
            let oc = |field: &str| -> Option<f64> {
                ohlc.and_then(|o| o.get(field))
                    .and_then(serde_json::Value::as_f64)
            };
            // Kite reports prices in rupees; the display type is f64 rupees.
            let ltp = f64_at("last_price").unwrap_or(0.0);
            if ltp <= 0.0 {
                continue;
            }
            out.push(Sourced::new(
                Quote {
                    id: id.clone(),
                    name: row
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .filter(|n| !n.is_empty())
                        .map(str::to_owned),
                    ltp,
                    prev_close: oc("close").unwrap_or(0.0),
                    open: oc("open"),
                    high: oc("high"),
                    low: oc("low"),
                    volume: f64_at("volume").map(|v| v as u64),
                    week52_high: f64_at("52_week_high"),
                    week52_low: f64_at("52_week_low"),
                    upper_band: None,
                    lower_band: None,
                },
                Provenance::new(ProviderId::Kite, FeedKind::Live, now, now),
            ));
        }
        if out.is_empty() {
            return Err(DataError::NotFound);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider(uri: String) -> KiteQuoteProvider {
        let rest = Arc::new(KiteRest::new(Some(uri), "testkey".into()).unwrap());
        rest.set_token(secrecy::SecretString::from("day-token"));
        KiteQuoteProvider::new(rest)
    }

    fn id(symbol: &str) -> InstrumentId {
        InstrumentId::new(Exchange::Nse, symbol).unwrap()
    }

    const QUOTE: &str = r#"{"status":"success","data":{
        "NSE:RELIANCE": {
            "name": "Reliance Industries",
            "last_price": 1375.6,
            "volume": 1234567,
            "ohlc": {"open": 1360.0, "high": 1380.0, "low": 1355.0, "close": 1350.0},
            "52_week_high": 1600.0,
            "52_week_low": 1100.0
        }
    }}"#;

    #[tokio::test]
    async fn quotes_map_to_live_provenance() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/quote"))
            .and(query_param("i", "NSE:RELIANCE"))
            .respond_with(ResponseTemplate::new(200).set_body_string(QUOTE))
            .mount(&server)
            .await;
        let quotes = provider(server.uri())
            .quotes(&[id("RELIANCE")])
            .await
            .unwrap();
        assert_eq!(quotes.len(), 1);
        let q = &quotes[0];
        assert_eq!(q.data.ltp, 1375.6);
        assert_eq!(q.data.prev_close, 1350.0);
        assert_eq!(q.data.volume, Some(1_234_567));
        assert_eq!(q.provenance.feed, FeedKind::Live);
        assert_eq!(q.provenance.provider, ProviderId::Kite);
    }

    #[tokio::test]
    async fn a_dead_session_surfaces_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403).set_body_string(
                r#"{"status":"error","message":"expired","error_type":"TokenException"}"#,
            ))
            .mount(&server)
            .await;
        let err = provider(server.uri())
            .quotes(&[id("RELIANCE")])
            .await
            .unwrap_err();
        assert!(matches!(err, DataError::Unauthorized));
    }

    #[tokio::test]
    async fn unknown_symbols_are_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"{"status":"success","data":{}}"#),
            )
            .mount(&server)
            .await;
        let err = provider(server.uri())
            .quotes(&[id("RELIANCE")])
            .await
            .unwrap_err();
        assert!(matches!(err, DataError::NotFound));
    }
}
