use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension as _, params};

use crate::{StoreError, migrations};

/// The main window's geometry, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowState {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

/// Handle to the database. Cheap to share behind an `Arc`.
#[derive(Debug)]
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Opens (creating if needed) the database at `path`, enables WAL and runs
    /// migrations. Creates the parent directory.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        // `journal_mode` returns a row, so it can't go through execute.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
    }

    /// A private in-memory database, for tests.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self, StoreError> {
        // WAL + NORMAL is durable across app crashes, and loses at most the
        // last transactions on power loss, which is fine for UI state.
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        migrations::migrate(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub(crate) fn lock(&self) -> Result<MutexGuard<'_, Connection>, StoreError> {
        self.conn.lock().map_err(|_| StoreError::Poisoned)
    }

    /// Current schema version.
    pub fn schema_version(&self) -> Result<u32, StoreError> {
        migrations::current_version(&*self.lock()?)
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .lock()?
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.lock()?.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value, updated_at = unixepoch()",
            params![key, value],
        )?;
        Ok(())
    }

    /// The most recent `limit` commands, oldest first.
    pub fn command_history(&self, limit: usize) -> Result<Vec<String>, StoreError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT command FROM (
                 SELECT id, command FROM command_history ORDER BY id DESC LIMIT ?1
             ) ORDER BY id ASC",
        )?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = stmt.query_map([limit], |r| r.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Replaces the whole history with `commands` (oldest first), atomically.
    pub fn replace_command_history(&self, commands: &[String]) -> Result<(), StoreError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM command_history", [])?;
        {
            let mut insert = tx.prepare("INSERT INTO command_history (command) VALUES (?1)")?;
            for command in commands {
                insert.execute([command])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn window_state(&self) -> Result<Option<WindowState>, StoreError> {
        Ok(self
            .lock()?
            .query_row(
                "SELECT x, y, width, height, maximized FROM window_state WHERE id = 1",
                [],
                |r| {
                    Ok(WindowState {
                        x: r.get(0)?,
                        y: r.get(1)?,
                        width: r.get(2)?,
                        height: r.get(3)?,
                        maximized: r.get::<_, i64>(4)? != 0,
                    })
                },
            )
            .optional()?)
    }

    pub fn set_window_state(&self, state: WindowState) -> Result<(), StoreError> {
        self.lock()?.execute(
            "INSERT INTO window_state (id, x, y, width, height, maximized)
             VALUES (1, ?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (id) DO UPDATE SET
                 x = excluded.x, y = excluded.y, width = excluded.width,
                 height = excluded.height, maximized = excluded.maximized",
            params![
                state.x,
                state.y,
                state.width,
                state.height,
                i64::from(state.maximized)
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LATEST_VERSION;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    #[test]
    fn settings_round_trip_and_overwrite() {
        let s = store();
        assert_eq!(s.setting("ui.layout").unwrap(), None);
        s.set_setting("ui.layout", "2").unwrap();
        assert_eq!(s.setting("ui.layout").unwrap().as_deref(), Some("2"));
        s.set_setting("ui.layout", "4").unwrap();
        assert_eq!(s.setting("ui.layout").unwrap().as_deref(), Some("4"));
    }

    #[test]
    fn history_keeps_order_and_limit() {
        let s = store();
        let cmds: Vec<String> = ["A", "B", "C", "D"].map(String::from).to_vec();
        s.replace_command_history(&cmds).unwrap();
        assert_eq!(s.command_history(10).unwrap(), cmds);
        assert_eq!(s.command_history(2).unwrap(), ["C", "D"]);
        s.replace_command_history(&["X".to_owned()]).unwrap();
        assert_eq!(s.command_history(10).unwrap(), ["X"]);
        s.replace_command_history(&[]).unwrap();
        assert!(s.command_history(10).unwrap().is_empty());
    }

    #[test]
    fn window_state_round_trips_including_negative_origin() {
        let s = store();
        assert_eq!(s.window_state().unwrap(), None);
        let a = WindowState {
            x: -1920,
            y: 40,
            width: 1600,
            height: 900,
            maximized: false,
        };
        s.set_window_state(a).unwrap();
        assert_eq!(s.window_state().unwrap(), Some(a));
        let b = WindowState {
            maximized: true,
            ..a
        };
        s.set_window_state(b).unwrap();
        assert_eq!(s.window_state().unwrap(), Some(b));
    }

    #[test]
    fn file_database_uses_wal_and_persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/nivesh.db");
        {
            let s = Store::open(&path).unwrap();
            s.set_setting("k", "v").unwrap();
            let mode: String = s
                .lock()
                .unwrap()
                .query_row("PRAGMA journal_mode", [], |r| r.get(0))
                .unwrap();
            assert_eq!(mode, "wal");
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(s.schema_version().unwrap(), LATEST_VERSION);
        assert_eq!(s.setting("k").unwrap().as_deref(), Some("v"));
    }

    #[test]
    fn opening_a_newer_database_fails_without_modifying_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nivesh.db");
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", LATEST_VERSION + 5)
            .unwrap();
        assert!(matches!(
            Store::open(&path),
            Err(StoreError::SchemaTooNew { .. })
        ));
    }

    #[test]
    fn window_state_rejects_zero_size() {
        let s = store();
        let bad = WindowState {
            x: 0,
            y: 0,
            width: 0,
            height: 10,
            maximized: false,
        };
        assert!(s.set_window_state(bad).is_err());
    }
}
