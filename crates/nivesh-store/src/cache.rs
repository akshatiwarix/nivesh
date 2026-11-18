//! A persistent key/value cache for market data.
//!
//! Payloads are opaque JSON text; the data layer decides what they mean. Rows
//! exist so that after a restart with no network the UI can still show the last
//! known data, flagged STALE.

use rusqlite::{OptionalExtension as _, params};

use crate::{Store, StoreError};

/// One cached payload and when it was fetched (UTC epoch milliseconds).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheRow {
    pub payload: String,
    pub fetched_at_ms: i64,
}

impl Store {
    pub fn cache_get(&self, key: &str) -> Result<Option<CacheRow>, StoreError> {
        let conn = self.lock()?;
        Ok(conn
            .query_row(
                "SELECT payload, fetched_at FROM data_cache WHERE key = ?1",
                params![key],
                |row| {
                    Ok(CacheRow {
                        payload: row.get(0)?,
                        fetched_at_ms: row.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    /// Inserts or replaces `key`. `kind` groups rows for pruning (`quote`,
    /// `index`, `history`).
    pub fn cache_put(
        &self,
        key: &str,
        kind: &str,
        payload: &str,
        fetched_at_ms: i64,
    ) -> Result<(), StoreError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO data_cache (key, kind, payload, fetched_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(key) DO UPDATE SET
                kind = excluded.kind,
                payload = excluded.payload,
                fetched_at = excluded.fetched_at",
            params![key, kind, payload, fetched_at_ms],
        )?;
        Ok(())
    }

    /// Deletes rows fetched before `cutoff_ms`. Returns how many were removed.
    pub fn cache_prune(&self, cutoff_ms: i64) -> Result<usize, StoreError> {
        let conn = self.lock()?;
        Ok(conn.execute(
            "DELETE FROM data_cache WHERE fetched_at < ?1",
            params![cutoff_ms],
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_get_and_overwrite() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.cache_get("q:NSE:TCS").unwrap(), None);
        store
            .cache_put("q:NSE:TCS", "quote", "{\"a\":1}", 1000)
            .unwrap();
        assert_eq!(
            store.cache_get("q:NSE:TCS").unwrap(),
            Some(CacheRow {
                payload: "{\"a\":1}".into(),
                fetched_at_ms: 1000
            })
        );
        store
            .cache_put("q:NSE:TCS", "quote", "{\"a\":2}", 2000)
            .unwrap();
        let row = store.cache_get("q:NSE:TCS").unwrap().unwrap();
        assert_eq!(
            (row.payload.as_str(), row.fetched_at_ms),
            ("{\"a\":2}", 2000)
        );
    }

    #[test]
    fn prune_removes_only_old_rows() {
        let store = Store::open_in_memory().unwrap();
        store.cache_put("old", "quote", "1", 100).unwrap();
        store.cache_put("new", "quote", "2", 900).unwrap();
        assert_eq!(store.cache_prune(500).unwrap(), 1);
        assert!(store.cache_get("old").unwrap().is_none());
        assert!(store.cache_get("new").unwrap().is_some());
    }

    #[test]
    fn rows_persist_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.db");
        Store::open(&path)
            .unwrap()
            .cache_put("k", "history", "[]", 5)
            .unwrap();
        let row = Store::open(&path).unwrap().cache_get("k").unwrap().unwrap();
        assert_eq!(row.payload, "[]");
    }
}
