//! Live ticks over a broker's stream. `Tick` is a display datum, so prices
//! are f64 like `Quote`; the adapters batch them and the hub coalesces.

use nivesh_core::InstrumentId;

/// How much of a tick the subscriber wants. `Ltp` is the cheapest and what
/// quote views usually need; `Full` adds the five-level depth Kite streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickMode {
    Ltp,
    Quote,
    Full,
}

/// One tick: the newest known state of one instrument from the broker.
#[derive(Debug, Clone, PartialEq)]
pub struct Tick {
    pub id: InstrumentId,
    /// Last traded price.
    pub ltp: f64,
    /// Traded volume today, when the broker reports it.
    pub volume: Option<u64>,
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub close: Option<f64>,
    /// Epoch ms of the exchange's own timestamp, when it gives one.
    pub exchange_ts_ms: Option<i64>,
}

/// A batch of ticks delivered together; the receiver applies it in one frame.
#[derive(Debug, Clone, PartialEq)]
pub struct TickBatch {
    pub ticks: Vec<Tick>,
}

/// A live subscription. Dropping it (or its `Receiver`) is how the consumer
/// unsubscribes; the adapter stops feeding a channel nobody reads.
pub struct TickSubscription {
    receiver: tokio::sync::mpsc::Receiver<TickBatch>,
}

impl TickSubscription {
    pub fn new(receiver: tokio::sync::mpsc::Receiver<TickBatch>) -> Self {
        Self { receiver }
    }

    /// The next batch, or `None` once the stream has closed.
    pub async fn next(&mut self) -> Option<TickBatch> {
        self.receiver.recv().await
    }
}
