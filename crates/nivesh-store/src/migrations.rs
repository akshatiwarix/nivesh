//! Versioned schema migrations, tracked in `PRAGMA user_version`.
//!
//! Append only: never edit or reorder an existing migration, because it may
//! already have run on a user's machine. Add a new entry instead.

use rusqlite::Connection;

use crate::StoreError;

/// `MIGRATIONS[i]` upgrades the schema from version `i` to `i + 1`.
const MIGRATIONS: &[&str] = &[
    // v1: settings, command history, window geometry.
    "
    CREATE TABLE settings (
        key        TEXT PRIMARY KEY,
        value      TEXT NOT NULL,
        updated_at INTEGER NOT NULL DEFAULT (unixepoch())
    ) STRICT;

    CREATE TABLE command_history (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        command    TEXT NOT NULL,
        created_at INTEGER NOT NULL DEFAULT (unixepoch())
    ) STRICT;

    -- A single row (id = 1): the main window's last geometry.
    CREATE TABLE window_state (
        id        INTEGER PRIMARY KEY CHECK (id = 1),
        x         INTEGER NOT NULL,
        y         INTEGER NOT NULL,
        width     INTEGER NOT NULL CHECK (width > 0),
        height    INTEGER NOT NULL CHECK (height > 0),
        maximized INTEGER NOT NULL CHECK (maximized IN (0, 1))
    ) STRICT;
    ",
    // v2: persistent data cache (quotes, index snapshots, candle series),
    // served as STALE when providers are unreachable.
    "
    CREATE TABLE data_cache (
        key        TEXT PRIMARY KEY,
        kind       TEXT NOT NULL,
        payload    TEXT NOT NULL,
        fetched_at INTEGER NOT NULL
    ) STRICT;

    CREATE INDEX data_cache_kind_fetched ON data_cache (kind, fetched_at);
    ",
    // v3: the instrument master (listed equities), refreshed daily.
    "
    CREATE TABLE instruments (
        exchange TEXT NOT NULL,
        symbol   TEXT NOT NULL,
        name     TEXT NOT NULL,
        isin     TEXT,
        series   TEXT,
        PRIMARY KEY (exchange, symbol)
    ) STRICT, WITHOUT ROWID;
    ",
    // v4: watchlists. A fresh database starts with one list so `WL ADD` works
    // straight away.
    "
    CREATE TABLE watchlists (
        id       INTEGER PRIMARY KEY AUTOINCREMENT,
        name     TEXT NOT NULL UNIQUE COLLATE NOCASE,
        position INTEGER NOT NULL
    ) STRICT;

    CREATE TABLE watchlist_items (
        list_id  INTEGER NOT NULL REFERENCES watchlists (id) ON DELETE CASCADE,
        exchange TEXT NOT NULL,
        symbol   TEXT NOT NULL,
        position INTEGER NOT NULL,
        PRIMARY KEY (list_id, exchange, symbol)
    ) STRICT, WITHOUT ROWID;

    INSERT INTO watchlists (name, position) VALUES ('WATCHLIST', 0);
    ",
    // v5: broker instrument tokens (Part 4). One row per instrument the
    // broker streams, refreshed daily from the broker's instrument dump. The
    // broker column keeps room for more adapters than Kite.
    "
    CREATE TABLE broker_tokens (
        broker           TEXT NOT NULL,
        instrument_token INTEGER NOT NULL,
        exchange         TEXT NOT NULL,
        symbol           TEXT NOT NULL,
        kind             TEXT NOT NULL,
        tick_size        REAL NOT NULL,
        lot_size         INTEGER NOT NULL,
        PRIMARY KEY (broker, instrument_token)
    ) STRICT, WITHOUT ROWID;

    CREATE INDEX broker_tokens_instrument
        ON broker_tokens (broker, exchange, symbol);
    ",
];

/// The schema version a fully migrated database has.
pub const LATEST_VERSION: u32 = MIGRATIONS.len() as u32;

pub(crate) fn current_version(conn: &Connection) -> Result<u32, StoreError> {
    Ok(conn.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

/// Brings `conn` up to [`LATEST_VERSION`]. Each migration runs in its own
/// transaction together with its version bump, so a failure leaves the
/// database at the last good version.
pub(crate) fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
    let found = current_version(conn)?;
    if found > LATEST_VERSION {
        return Err(StoreError::SchemaTooNew {
            found,
            supported: LATEST_VERSION,
        });
    }
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(found as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        // `index + 1` is at most MIGRATIONS.len(), which fits in u32.
        tx.pragma_update(
            None,
            "user_version",
            u32::try_from(index + 1).unwrap_or(u32::MAX),
        )?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn fresh_database_migrates_to_latest() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert_eq!(current_version(&conn).unwrap(), 0);
        migrate(&mut conn).unwrap();
        assert_eq!(current_version(&conn).unwrap(), LATEST_VERSION);
        assert_eq!(
            tables(&conn),
            [
                "broker_tokens",
                "command_history",
                "data_cache",
                "instruments",
                "settings",
                "watchlist_items",
                "watchlists",
                "window_state"
            ]
        );
    }

    #[test]
    fn migrating_twice_is_a_no_op() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        conn.execute("INSERT INTO settings (key, value) VALUES ('k', 'v')", [])
            .unwrap();
        migrate(&mut conn).unwrap();
        let n: i64 = conn
            .query_row("SELECT count(*) FROM settings", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn refuses_a_database_from_the_future() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", LATEST_VERSION + 1)
            .unwrap();
        let err = migrate(&mut conn).unwrap_err();
        assert!(matches!(
            err,
            StoreError::SchemaTooNew { found, supported }
                if found == LATEST_VERSION + 1 && supported == LATEST_VERSION
        ));
        // Nothing was touched.
        assert!(tables(&conn).is_empty());
    }

    #[test]
    fn failed_migration_rolls_back_and_keeps_the_version() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        // Simulate a broken later migration run by hand.
        let tx = conn.transaction().unwrap();
        assert!(
            tx.execute_batch("CREATE TABLE extra (a INTEGER); CREATE TABLE settings (x TEXT);")
                .is_err()
        );
        drop(tx); // rolls back
        assert!(!tables(&conn).contains(&"extra".to_owned()));
        assert_eq!(current_version(&conn).unwrap(), LATEST_VERSION);
    }

    #[test]
    fn migrations_pass_sqlite_integrity_check() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let ok: String = conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ok, "ok");
    }
}
