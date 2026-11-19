//! The broker instrument token map: which broker-side id streams which
//! instrument. One table per broker (`broker_tokens`), rebuilt wholesale
//! from the broker's daily instrument dump.

use crate::Store;
use crate::error::StoreError;
use rusqlite::OptionalExtension as _;

/// One row of the map: the broker's streaming id for one instrument.
#[derive(Debug, Clone, PartialEq)]
pub struct BrokerTokenRow {
    pub instrument_token: i64,
    pub exchange: String,
    pub symbol: String,
    /// `EQ`, `INDEX`, ... as the broker names the kind.
    pub kind: String,
    pub tick_size: f64,
    pub lot_size: i64,
}

impl Store {
    /// Replaces every row for `broker` with `rows`, in one transaction. A
    /// partial dump never replaces a good one: an empty `rows` with
    /// `replace` still clears, but an error leaves the old map intact.
    pub fn replace_broker_tokens(
        &self,
        broker: &str,
        rows: &[BrokerTokenRow],
    ) -> Result<usize, StoreError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "INSERT INTO broker_tokens
                 (broker, instrument_token, exchange, symbol, kind, tick_size, lot_size)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (broker, instrument_token)
             DO UPDATE SET exchange = ?3, symbol = ?4, kind = ?5,
                           tick_size = ?6, lot_size = ?7",
        )?;
        let mut done = 0usize;
        for row in rows {
            let n = stmt.execute((
                broker,
                row.instrument_token,
                &row.exchange,
                &row.symbol,
                &row.kind,
                row.tick_size,
                row.lot_size,
            ))?;
            done += n;
        }
        Ok(done)
    }

    /// The instrument streaming under `token`, if the map knows it.
    pub fn instrument_for_token(
        &self,
        broker: &str,
        instrument_token: i64,
    ) -> Result<Option<(String, String)>, StoreError> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT exchange, symbol FROM broker_tokens
             WHERE broker = ?1 AND instrument_token = ?2",
            (broker, instrument_token),
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(StoreError::from)
    }

    /// The broker's streaming id for one instrument, if mapped.
    pub fn token_for_instrument(
        &self,
        broker: &str,
        exchange: &str,
        symbol: &str,
    ) -> Result<Option<i64>, StoreError> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT instrument_token FROM broker_tokens
             WHERE broker = ?1 AND exchange = ?2 AND symbol = ?3
             LIMIT 1",
            (broker, exchange, symbol),
            |row| row.get(0),
        )
        .optional()
        .map_err(StoreError::from)
    }

    /// How many instruments are mapped for `broker` (health for the status bar).
    pub fn broker_token_count(&self, broker: &str) -> Result<i64, StoreError> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT count(*) FROM broker_tokens WHERE broker = ?1",
            (broker,),
            |row| row.get(0),
        )
        .map_err(StoreError::from)
    }

    /// Drops every row for `broker` (e.g. on logout, to keep the store clean).
    pub fn clear_broker_tokens(&self, broker: &str) -> Result<(), StoreError> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM broker_tokens WHERE broker = ?1", (broker,))?;
        Ok(())
    }
}
