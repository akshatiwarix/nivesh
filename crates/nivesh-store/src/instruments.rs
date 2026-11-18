//! The instrument master table.

use rusqlite::params;

use crate::{Store, StoreError};

/// One listed security. `exchange` is the code (`NSE`, `BSE`); this crate
/// doesn't depend on the domain types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstrumentRow {
    pub exchange: String,
    pub symbol: String,
    pub name: String,
    pub isin: Option<String>,
    pub series: Option<String>,
}

impl Store {
    /// Replaces every row of `exchange` with `rows`, atomically, so a refresh
    /// never leaves a half-updated list.
    pub fn replace_instruments(
        &self,
        exchange: &str,
        rows: &[InstrumentRow],
    ) -> Result<(), StoreError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM instruments WHERE exchange = ?1",
            params![exchange],
        )?;
        {
            let mut insert = tx.prepare(
                "INSERT OR REPLACE INTO instruments (exchange, symbol, name, isin, series)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for row in rows {
                insert.execute(params![
                    exchange, row.symbol, row.name, row.isin, row.series
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// All instruments, ordered by exchange then symbol.
    pub fn instruments(&self) -> Result<Vec<InstrumentRow>, StoreError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT exchange, symbol, name, isin, series FROM instruments ORDER BY exchange, symbol",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(InstrumentRow {
                    exchange: r.get(0)?,
                    symbol: r.get(1)?,
                    name: r.get(2)?,
                    isin: r.get(3)?,
                    series: r.get(4)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(exchange: &str, symbol: &str) -> InstrumentRow {
        InstrumentRow {
            exchange: exchange.into(),
            symbol: symbol.into(),
            name: format!("{symbol} Ltd"),
            isin: Some("INE000000000".into()),
            series: Some("EQ".into()),
        }
    }

    #[test]
    fn replace_is_per_exchange_and_atomic() {
        let store = Store::open_in_memory().unwrap();
        store
            .replace_instruments("NSE", &[row("NSE", "A"), row("NSE", "B")])
            .unwrap();
        store
            .replace_instruments("BSE", &[row("BSE", "500325")])
            .unwrap();
        store
            .replace_instruments("NSE", &[row("NSE", "C")])
            .unwrap();
        let all = store.instruments().unwrap();
        let ids: Vec<_> = all
            .iter()
            .map(|r| (r.exchange.as_str(), r.symbol.as_str()))
            .collect();
        assert_eq!(ids, [("BSE", "500325"), ("NSE", "C")]);
    }

    #[test]
    fn duplicate_symbols_in_one_batch_do_not_fail() {
        let store = Store::open_in_memory().unwrap();
        store
            .replace_instruments("NSE", &[row("NSE", "A"), row("NSE", "A")])
            .unwrap();
        assert_eq!(store.instruments().unwrap().len(), 1);
    }

    #[test]
    fn optional_fields_round_trip_as_null() {
        let store = Store::open_in_memory().unwrap();
        let mut r = row("NSE", "A");
        r.isin = None;
        r.series = None;
        store
            .replace_instruments("NSE", std::slice::from_ref(&r))
            .unwrap();
        assert_eq!(store.instruments().unwrap(), vec![r]);
    }
}
