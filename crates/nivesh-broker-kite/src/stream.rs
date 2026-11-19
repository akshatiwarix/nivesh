//! The Kite ticker as a [`MarketStream`]: one WebSocket to
//! `wss://ws.kite.trade`, subscriptions reference-counted by the hub, ticks
//! resolved through the token map and batched into a channel.
//!
//! The connection is owned by a task that reconnects with backoff and
//! resubscribes everything after a drop (the daily token change or a
//! network cut). Up to 3000 instruments per connection, per the docs;
//! the hub only ever subscribes what views show.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use nivesh_broker::{BrokerError, MarketStream, Tick, TickBatch, TickMode, TickSubscription};
use nivesh_core::InstrumentId;
use nivesh_store::Store;
use secrecy::ExposeSecret;
use tokio_tungstenite::tungstenite::Message;

use crate::dump::KITE;
use crate::parser;
use crate::rest::KiteRest;

pub const TICKER_URL: &str = "wss://ws.kite.trade";

/// Reconnect waits grow by this each failure, capped at the max.
const RECONNECT_BACKOFF: Duration = Duration::from_secs(3);
const RECONNECT_MAX: Duration = Duration::from_secs(60);

/// How many instruments one connection may carry, per the docs.
const INSTRUMENTS_PER_CONNECTION: usize = 3000;

pub struct KiteStream {
    rest: Arc<KiteRest>,
    store: Arc<Store>,
}

impl KiteStream {
    pub fn new(rest: Arc<KiteRest>, store: Arc<Store>) -> Self {
        Self { rest, store }
    }

    /// The endpoint with the session's credentials; the token changes daily,
    /// so it is read at every (re)connect.
    fn ticker_url(&self) -> Result<url::Url, BrokerError> {
        let token = self.rest.access_token().ok_or(BrokerError::Unauthorized)?;
        let mut url = url::Url::parse(TICKER_URL)
            .map_err(|e| BrokerError::InvalidRequest(format!("ticker url: {e}")))?;
        url.query_pairs_mut()
            .append_pair("api_key", &self.rest.api_key());
        // The token is in the URL query; never log this URL.
        url.query_pairs_mut()
            .append_pair("access_token", token.expose_secret());
        Ok(url)
    }

    /// Subscribes `ids` and runs the read loop, feeding `sender`. Resolves
    /// tokens through the map; unknown tokens are skipped. Returns when the
    /// socket closes for good or `sender` is dropped. `backoff` carries the
    /// reconnect delay across restarts so repeated failures back off.
    async fn run(
        &self,
        ids: Vec<InstrumentId>,
        mode: TickMode,
        sender: tokio::sync::mpsc::Sender<TickBatch>,
        backoff: Duration,
    ) -> Result<(), BrokerError> {
        let started = std::time::Instant::now();
        // The URL string is never logged (it carries the access token).
        let url = self.ticker_url()?;
        let (ws, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .map_err(|e| BrokerError::Upstream(format!("ticker connect: {e}")))?;
        let (mut sink, mut source) = ws.split();

        // Subscribe first, then set the mode (the docs' own order).
        let tokens: Vec<i64> = self
            .token_map(&ids)
            .iter()
            .map(|(token, _)| *token)
            .collect();
        if tokens.len() > INSTRUMENTS_PER_CONNECTION {
            return Err(BrokerError::InvalidRequest(format!(
                "at most {INSTRUMENTS_PER_CONNECTION} instruments per ticker connection"
            )));
        }
        let subscribe = format!(
            r#"{{"a":"subscribe","v":{}}}"#,
            serde_json::to_string(&tokens).unwrap_or_else(|_| "[]".into())
        );
        sink.send(Message::Text(subscribe.into()))
            .await
            .map_err(|e| BrokerError::Upstream(format!("ticker subscribe: {e}")))?;
        let mode_text = match mode {
            TickMode::Ltp => "ltp",
            TickMode::Quote => "quote",
            TickMode::Full => "full",
        };
        let set_mode = format!(
            r#"{{"a":"mode","v":["{mode_text}",{}]}}"#,
            serde_json::to_string(&tokens).unwrap_or_else(|_| "[]".into())
        );
        sink.send(Message::Text(set_mode.into()))
            .await
            .map_err(|e| BrokerError::Upstream(format!("ticker mode: {e}")))?;

        let by_token: std::collections::HashMap<i64, InstrumentId> =
            self.token_map(&ids).into_iter().collect();
        while let Some(message) = source.next().await {
            match message {
                Ok(Message::Binary(bytes)) => {
                    if bytes.len() <= 1 {
                        // The documented 1-byte heartbeat.
                        continue;
                    }
                    match parser::parse_message(&bytes) {
                        Ok(raw) => {
                            let ticks: Vec<Tick> = raw
                                .into_iter()
                                .filter_map(|r| {
                                    let id = by_token.get(&r.instrument_token).cloned()?;
                                    Some(Tick {
                                        id,
                                        ltp: r.ltp,
                                        volume: r.volume,
                                        open: r.open,
                                        high: r.high,
                                        low: r.low,
                                        close: r.close,
                                        exchange_ts_ms: r.exchange_ts_ms,
                                    })
                                })
                                .collect();
                            if !ticks.is_empty() && sender.send(TickBatch { ticks }).await.is_err()
                            {
                                // The receiver is gone: stop streaming.
                                return Ok(());
                            }
                        }
                        Err(e) => {
                            tracing::warn!(%e, "unusable tick frame; carrying on");
                        }
                    }
                }
                Ok(Message::Text(text)) => {
                    // Postbacks and errors arrive as JSON text; order updates
                    // matter from Part 6. Logged without the payload for now.
                    tracing::debug!(len = text.len(), "ticker text message");
                }
                Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {}
                Ok(Message::Close(_)) | Err(_) => {
                    // A dropped socket: reconnect and resubscribe everything.
                    // A connection that lasted a while resets the backoff.
                    let next = if started.elapsed() > Duration::from_secs(60) {
                        RECONNECT_BACKOFF
                    } else {
                        (backoff * 2).min(RECONNECT_MAX)
                    };
                    return Box::pin(self.run(ids, mode, sender, next)).await;
                }
            }
        }
        Ok(())
    }

    /// Token ↔ instrument for exactly the ids requested, in request order.
    fn token_map(&self, ids: &[InstrumentId]) -> Vec<(i64, InstrumentId)> {
        ids.iter()
            .filter_map(|id| {
                self.store
                    .token_for_instrument(KITE, id.exchange().code(), id.symbol())
                    .ok()
                    .flatten()
                    .map(|token| (token, id.clone()))
            })
            .collect()
    }
}

#[async_trait]
impl MarketStream for KiteStream {
    async fn subscribe(
        &self,
        ids: &[InstrumentId],
        mode: TickMode,
    ) -> Result<TickSubscription, BrokerError> {
        let (sender, receiver) = tokio::sync::mpsc::channel(64);
        let this = self.clone_for_run();
        let ids = ids.to_vec();
        tokio::spawn(async move {
            if let Err(e) = this.run(ids, mode, sender, RECONNECT_BACKOFF).await {
                tracing::warn!(%e, "ticker stream ended");
            }
        });
        Ok(TickSubscription::new(receiver))
    }
}

impl KiteStream {
    /// The read loop outlives the subscription call; the shared state is
    /// cheap to clone (`Arc`s under the hood).
    fn clone_for_run(&self) -> Self {
        Self {
            rest: Arc::clone(&self.rest),
            store: Arc::clone(&self.store),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::encode_message;

    #[tokio::test]
    async fn the_ticker_url_carries_the_session() {
        let rest = Arc::new(KiteRest::new(None, "key1".into()).unwrap());
        let store = Arc::new(nivesh_store::Store::open_in_memory().unwrap());
        let stream = KiteStream::new(rest.clone(), store);
        assert_eq!(stream.ticker_url().unwrap_err(), BrokerError::Unauthorized);
        rest.set_token(secrecy::SecretString::from("day-token"));
        let url = stream.ticker_url().unwrap();
        assert_eq!(url.host_str(), Some("ws.kite.trade"));
        assert!(url.query().unwrap().contains("api_key=key1"));
        assert!(url.query().unwrap().contains("access_token=day-token"));
    }

    #[test]
    fn a_multi_packet_message_parses_whole() {
        // One 8-byte LTP packet and one 28-byte index packet.
        let frame = encode_message(&[
            &[738561, 137_560],
            &[
                16, 2_551_435, 2_556_000, 2_548_000, 2_550_000, 2_549_000, 2_300,
            ],
        ]);
        let ticks = parser::parse_message(&frame).unwrap();
        assert_eq!(ticks.len(), 2);
        assert_eq!(ticks[0].instrument_token, 738561);
        assert_eq!(ticks[0].ltp, 1375.6);
        assert_eq!(ticks[1].ltp, 25514.35);
        assert_eq!(ticks[1].high, Some(25560.0));
        assert_eq!(ticks[1].close, Some(25490.0));
    }

    #[test]
    fn a_quote_packet_carries_the_day_fields() {
        // 44 bytes = 11 int32 fields.
        let frame = encode_message(&[&[
            738561,  // token
            137_560, // ltp
            5,       // ltq
            137_100, // avg price
            123_456, // volume
            10,      // buy qty
            20,      // sell qty
            136_000, // open
            138_000, // high
            135_500, // low
            135_000, // close
        ]]);
        let ticks = parser::parse_message(&frame).unwrap();
        assert_eq!(ticks.len(), 1);
        let t = &ticks[0];
        assert_eq!(t.ltp, 1375.6);
        assert_eq!(t.volume, Some(123_456));
        assert_eq!(t.open, Some(1360.0));
        assert_eq!(t.high, Some(1380.0));
        assert_eq!(t.low, Some(1355.0));
        assert_eq!(t.close, Some(1350.0));
    }

    #[test]
    fn an_arbitrary_frame_is_an_error_never_a_panic() {
        assert!(
            parser::parse_message(&[0, 0]).is_ok(),
            "zero packets is fine"
        );
        assert!(parser::parse_message(&[1, 0]).is_err());
        assert!(parser::parse_message(&[1, 0, 99, 0, 1, 2, 3]).is_err());
        assert!(parser::parse_message(&[]).is_err());
        assert!(parser::parse_message(&[1, 0, 10, 0, 1, 2]).is_err());
    }
}
