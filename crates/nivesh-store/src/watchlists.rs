//! Named watchlists of instruments, in user-chosen order.

use rusqlite::{Connection, OptionalExtension as _, params};

use crate::{Store, StoreError};

pub const MAX_LISTS: usize = 20;
pub const MAX_ITEMS_PER_LIST: usize = 200;
pub const MAX_NAME_LEN: usize = 32;

const ACTIVE_KEY: &str = "watchlist.active";

/// An instrument in a watchlist. `exchange` is the code (`NSE`, `BSE`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchItem {
    pub exchange: String,
    pub symbol: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watchlist {
    pub id: i64,
    pub name: String,
    pub items: Vec<WatchItem>,
}

fn clean_name(name: &str) -> Result<String, StoreError> {
    let name = name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase();
    if name.is_empty() {
        return Err(StoreError::Invalid("a watchlist needs a name".into()));
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Err(StoreError::Invalid(format!(
            "watchlist names are limited to {MAX_NAME_LEN} characters"
        )));
    }
    Ok(name)
}

fn list_exists(conn: &Connection, id: i64) -> Result<bool, StoreError> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM watchlists WHERE id = ?1",
            params![id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn item_symbols(conn: &Connection, list_id: i64) -> Result<Vec<WatchItem>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT exchange, symbol FROM watchlist_items WHERE list_id = ?1 ORDER BY position",
    )?;
    let items = stmt
        .query_map(params![list_id], |r| {
            Ok(WatchItem {
                exchange: r.get(0)?,
                symbol: r.get(1)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(items)
}

impl Store {
    /// Every list with its items, in list order.
    pub fn watchlists(&self) -> Result<Vec<Watchlist>, StoreError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare("SELECT id, name FROM watchlists ORDER BY position, id")?;
        let heads: Vec<(i64, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        heads
            .into_iter()
            .map(|(id, name)| {
                Ok(Watchlist {
                    id,
                    name,
                    items: item_symbols(&conn, id)?,
                })
            })
            .collect()
    }

    /// The list `WL ADD` goes to: the saved choice if it still exists,
    /// otherwise the first list.
    pub fn active_watchlist(&self) -> Result<i64, StoreError> {
        let saved = self
            .setting(ACTIVE_KEY)?
            .and_then(|v| v.parse::<i64>().ok());
        let conn = self.lock()?;
        if let Some(id) = saved.filter(|id| list_exists(&conn, *id).unwrap_or(false)) {
            return Ok(id);
        }
        conn.query_row(
            "SELECT id FROM watchlists ORDER BY position, id LIMIT 1",
            [],
            |r| r.get(0),
        )
        .map_err(Into::into)
    }

    pub fn set_active_watchlist(&self, id: i64) -> Result<(), StoreError> {
        {
            let conn = self.lock()?;
            if !list_exists(&conn, id)? {
                return Err(StoreError::NotFound);
            }
        }
        self.set_setting(ACTIVE_KEY, &id.to_string())
    }

    /// Creates a list (names are upper-cased and unique, ignoring case).
    pub fn create_watchlist(&self, name: &str) -> Result<i64, StoreError> {
        let name = clean_name(name)?;
        let conn = self.lock()?;
        let count: i64 = conn.query_row("SELECT count(*) FROM watchlists", [], |r| r.get(0))?;
        if count as usize >= MAX_LISTS {
            return Err(StoreError::Invalid(format!(
                "at most {MAX_LISTS} watchlists"
            )));
        }
        let taken = conn
            .query_row(
                "SELECT 1 FROM watchlists WHERE name = ?1",
                params![name],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if taken {
            return Err(StoreError::Invalid(format!(
                "a watchlist named {name} already exists"
            )));
        }
        conn.execute(
            "INSERT INTO watchlists (name, position)
             VALUES (?1, COALESCE((SELECT MAX(position) + 1 FROM watchlists), 0))",
            params![name],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn rename_watchlist(&self, id: i64, name: &str) -> Result<(), StoreError> {
        let name = clean_name(name)?;
        let conn = self.lock()?;
        let clash = conn
            .query_row(
                "SELECT id FROM watchlists WHERE name = ?1",
                params![name],
                |r| r.get::<_, i64>(0),
            )
            .optional()?;
        if clash.is_some_and(|other| other != id) {
            return Err(StoreError::Invalid(format!(
                "a watchlist named {name} already exists"
            )));
        }
        match conn.execute(
            "UPDATE watchlists SET name = ?1 WHERE id = ?2",
            params![name, id],
        )? {
            0 => Err(StoreError::NotFound),
            _ => Ok(()),
        }
    }

    /// Deletes a list and its items. The last list can't be deleted.
    pub fn delete_watchlist(&self, id: i64) -> Result<(), StoreError> {
        let conn = self.lock()?;
        let count: i64 = conn.query_row("SELECT count(*) FROM watchlists", [], |r| r.get(0))?;
        if !list_exists(&conn, id)? {
            return Err(StoreError::NotFound);
        }
        if count <= 1 {
            return Err(StoreError::Invalid(
                "the last watchlist can't be deleted".into(),
            ));
        }
        conn.execute("DELETE FROM watchlists WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Appends an instrument. Returns false if it was already in the list.
    pub fn add_watch_item(
        &self,
        list_id: i64,
        exchange: &str,
        symbol: &str,
    ) -> Result<bool, StoreError> {
        let conn = self.lock()?;
        if !list_exists(&conn, list_id)? {
            return Err(StoreError::NotFound);
        }
        let count: i64 = conn.query_row(
            "SELECT count(*) FROM watchlist_items WHERE list_id = ?1",
            params![list_id],
            |r| r.get(0),
        )?;
        let exists = conn
            .query_row(
                "SELECT 1 FROM watchlist_items WHERE list_id = ?1 AND exchange = ?2 AND symbol = ?3",
                params![list_id, exchange, symbol],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if exists {
            return Ok(false);
        }
        if count as usize >= MAX_ITEMS_PER_LIST {
            return Err(StoreError::Invalid(format!(
                "a watchlist holds at most {MAX_ITEMS_PER_LIST} symbols"
            )));
        }
        conn.execute(
            "INSERT INTO watchlist_items (list_id, exchange, symbol, position) VALUES (?1, ?2, ?3, ?4)",
            params![list_id, exchange, symbol, count],
        )?;
        Ok(true)
    }

    /// Removes an instrument and closes the gap. Returns false if it wasn't there.
    pub fn remove_watch_item(
        &self,
        list_id: i64,
        exchange: &str,
        symbol: &str,
    ) -> Result<bool, StoreError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let removed = tx.execute(
            "DELETE FROM watchlist_items WHERE list_id = ?1 AND exchange = ?2 AND symbol = ?3",
            params![list_id, exchange, symbol],
        )?;
        if removed > 0 {
            renumber(&tx, list_id)?;
        }
        tx.commit()?;
        Ok(removed > 0)
    }

    /// Moves an instrument `delta` places (negative = up), stopping at the ends.
    /// Returns its new index, or `None` if it isn't in the list.
    pub fn move_watch_item(
        &self,
        list_id: i64,
        exchange: &str,
        symbol: &str,
        delta: i32,
    ) -> Result<Option<usize>, StoreError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        let mut items = item_symbols(&tx, list_id)?;
        let Some(from) = items
            .iter()
            .position(|i| i.exchange == exchange && i.symbol == symbol)
        else {
            return Ok(None);
        };
        let to = (from as i64 + i64::from(delta)).clamp(0, items.len() as i64 - 1) as usize;
        let item = items.remove(from);
        items.insert(to, item);
        for (position, item) in items.iter().enumerate() {
            tx.execute(
                "UPDATE watchlist_items SET position = ?4 WHERE list_id = ?1 AND exchange = ?2 AND symbol = ?3",
                params![list_id, item.exchange, item.symbol, position as i64],
            )?;
        }
        tx.commit()?;
        Ok(Some(to))
    }
}

fn renumber(conn: &Connection, list_id: i64) -> Result<(), StoreError> {
    for (position, item) in item_symbols(conn, list_id)?.iter().enumerate() {
        conn.execute(
            "UPDATE watchlist_items SET position = ?4 WHERE list_id = ?1 AND exchange = ?2 AND symbol = ?3",
            params![list_id, item.exchange, item.symbol, position as i64],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh store whose default list is emptied of its starter symbols,
    /// so each test starts from nothing.
    fn store() -> Store {
        let s = Store::open_in_memory().unwrap();
        clear_default(&s);
        s
    }

    fn clear_default(s: &Store) {
        let id = s.active_watchlist().unwrap();
        for item in s
            .watchlists()
            .unwrap()
            .into_iter()
            .find(|l| l.id == id)
            .unwrap()
            .items
        {
            s.remove_watch_item(id, &item.exchange, &item.symbol)
                .unwrap();
        }
    }

    #[test]
    fn a_fresh_database_starts_with_ten_large_caps() {
        let s = Store::open_in_memory().unwrap();
        let id = s.active_watchlist().unwrap();
        let syms = symbols(&s, id);
        assert_eq!(syms.len(), 10);
        assert_eq!(syms[0], "RELIANCE");
        assert_eq!(syms[9], "MARUTI");
    }

    fn symbols(store: &Store, list: i64) -> Vec<String> {
        store
            .watchlists()
            .unwrap()
            .into_iter()
            .find(|l| l.id == list)
            .unwrap()
            .items
            .into_iter()
            .map(|i| i.symbol)
            .collect()
    }

    #[test]
    fn a_fresh_database_has_one_list_which_is_active() {
        let s = store();
        let lists = s.watchlists().unwrap();
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].name, "WATCHLIST");
        assert_eq!(s.active_watchlist().unwrap(), lists[0].id);
    }

    #[test]
    fn items_keep_insertion_order_and_reject_duplicates() {
        let s = store();
        let id = s.active_watchlist().unwrap();
        assert!(s.add_watch_item(id, "NSE", "TCS").unwrap());
        assert!(s.add_watch_item(id, "NSE", "INFY").unwrap());
        assert!(
            s.add_watch_item(id, "BSE", "TCS").unwrap(),
            "same symbol, other exchange"
        );
        assert!(!s.add_watch_item(id, "NSE", "TCS").unwrap());
        assert_eq!(symbols(&s, id), ["TCS", "INFY", "TCS"]);
    }

    #[test]
    fn remove_closes_the_gap() {
        let s = store();
        let id = s.active_watchlist().unwrap();
        for sym in ["A", "B", "C"] {
            s.add_watch_item(id, "NSE", sym).unwrap();
        }
        assert!(s.remove_watch_item(id, "NSE", "B").unwrap());
        assert!(!s.remove_watch_item(id, "NSE", "B").unwrap());
        assert_eq!(symbols(&s, id), ["A", "C"]);
        s.add_watch_item(id, "NSE", "D").unwrap();
        assert_eq!(
            symbols(&s, id),
            ["A", "C", "D"],
            "positions stay consistent"
        );
    }

    #[test]
    fn reordering_moves_by_delta_and_stops_at_the_ends() {
        let s = store();
        let id = s.active_watchlist().unwrap();
        for sym in ["A", "B", "C", "D"] {
            s.add_watch_item(id, "NSE", sym).unwrap();
        }
        assert_eq!(s.move_watch_item(id, "NSE", "C", -1).unwrap(), Some(1));
        assert_eq!(symbols(&s, id), ["A", "C", "B", "D"]);
        assert_eq!(
            s.move_watch_item(id, "NSE", "A", -1).unwrap(),
            Some(0),
            "clamped at the top"
        );
        assert_eq!(
            s.move_watch_item(id, "NSE", "D", 5).unwrap(),
            Some(3),
            "clamped at the bottom"
        );
        assert_eq!(s.move_watch_item(id, "NSE", "A", 2).unwrap(), Some(2));
        assert_eq!(symbols(&s, id), ["C", "B", "A", "D"]);
        assert_eq!(s.move_watch_item(id, "NSE", "ZZZ", 1).unwrap(), None);
    }

    #[test]
    fn lists_are_named_uppercase_unique_and_bounded() {
        let s = store();
        let tech = s.create_watchlist("  tech   stocks ").unwrap();
        assert_eq!(s.watchlists().unwrap()[1].name, "TECH STOCKS");
        for bad in ["", "   ", &"X".repeat(MAX_NAME_LEN + 1)] {
            assert!(
                matches!(s.create_watchlist(bad), Err(StoreError::Invalid(_))),
                "{bad:?}"
            );
        }
        assert!(
            matches!(s.create_watchlist("Tech Stocks"), Err(StoreError::Invalid(m)) if m.contains("already exists"))
        );
        assert!(matches!(
            s.rename_watchlist(tech, "watchlist"),
            Err(StoreError::Invalid(_))
        ));
        s.rename_watchlist(tech, "banks").unwrap();
        s.rename_watchlist(tech, "BANKS").unwrap(); // renaming to itself is fine
        assert_eq!(s.watchlists().unwrap()[1].name, "BANKS");
        assert!(matches!(
            s.rename_watchlist(999, "x"),
            Err(StoreError::NotFound)
        ));
    }

    #[test]
    fn the_number_of_lists_and_items_is_bounded() {
        let s = store();
        for n in 1..MAX_LISTS {
            s.create_watchlist(&format!("L{n}")).unwrap();
        }
        assert!(matches!(
            s.create_watchlist("ONE MORE"),
            Err(StoreError::Invalid(_))
        ));
        let id = s.active_watchlist().unwrap();
        for n in 0..MAX_ITEMS_PER_LIST {
            s.add_watch_item(id, "NSE", &format!("S{n}")).unwrap();
        }
        assert!(matches!(
            s.add_watch_item(id, "NSE", "EXTRA"),
            Err(StoreError::Invalid(_))
        ));
    }

    #[test]
    fn deleting_a_list_removes_its_items_but_never_the_last_list() {
        let s = store();
        let first = s.active_watchlist().unwrap();
        let second = s.create_watchlist("SECOND").unwrap();
        s.add_watch_item(second, "NSE", "TCS").unwrap();
        s.delete_watchlist(second).unwrap();
        assert_eq!(s.watchlists().unwrap().len(), 1);
        let orphans: i64 = s
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM watchlist_items WHERE list_id = ?1",
                params![second],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(orphans, 0, "items go with their list");
        assert!(matches!(
            s.delete_watchlist(first),
            Err(StoreError::Invalid(_))
        ));
        assert!(matches!(
            s.delete_watchlist(second),
            Err(StoreError::NotFound)
        ));
    }

    #[test]
    fn the_active_list_falls_back_when_it_is_deleted() {
        let s = store();
        let first = s.active_watchlist().unwrap();
        let second = s.create_watchlist("SECOND").unwrap();
        s.set_active_watchlist(second).unwrap();
        assert_eq!(s.active_watchlist().unwrap(), second);
        s.delete_watchlist(second).unwrap();
        assert_eq!(s.active_watchlist().unwrap(), first);
        assert!(matches!(
            s.set_active_watchlist(999),
            Err(StoreError::NotFound)
        ));
    }

    #[test]
    fn operations_on_a_missing_list_are_not_found() {
        let s = store();
        assert!(matches!(
            s.add_watch_item(999, "NSE", "TCS"),
            Err(StoreError::NotFound)
        ));
    }

    #[test]
    fn lists_items_order_and_the_active_choice_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.db");
        let (second, expected) = {
            let s = Store::open(&path).unwrap();
            clear_default(&s);
            let first = s.active_watchlist().unwrap();
            let second = s.create_watchlist("SECOND").unwrap();
            for sym in ["A", "B", "C"] {
                s.add_watch_item(first, "NSE", sym).unwrap();
            }
            s.move_watch_item(first, "NSE", "C", -2).unwrap();
            s.add_watch_item(second, "BSE", "500325").unwrap();
            s.set_active_watchlist(second).unwrap();
            (second, s.watchlists().unwrap())
        };
        let s = Store::open(&path).unwrap();
        assert_eq!(s.watchlists().unwrap(), expected);
        assert_eq!(s.active_watchlist().unwrap(), second);
        assert_eq!(symbols(&s, expected[0].id), ["C", "A", "B"]);
    }
}
