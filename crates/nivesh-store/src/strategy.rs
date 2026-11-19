//! Strategies, backtest runs and paper state in the store.

use crate::Store;
use crate::error::StoreError;
use rusqlite::OptionalExtension as _;

/// A saved strategy: the template it came from and its serialised rules.
#[derive(Debug, Clone, PartialEq)]
pub struct StrategyRow {
    pub id: i64,
    pub name: String,
    /// The template's name (`MA Crossover`, ...), for the builder.
    pub template: String,
    /// The serialised `RuleSet`.
    pub json: String,
    pub created_at: i64,
}

/// A saved backtest run.
#[derive(Debug, Clone, PartialEq)]
pub struct BacktestRow {
    pub id: i64,
    pub strategy_id: i64,
    /// The instrument, `NSE:RELIANCE` shaped.
    pub symbol: String,
    pub interval: String,
    pub range: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub total_pnl: Option<f64>,
    pub max_drawdown: Option<f64>,
    /// The serialised trade log.
    pub trades_json: Option<String>,
    /// The serialised walk-forward check, when the run had one.
    pub walk_forward_json: Option<String>,
    /// The serialised equity and drawdown curve.
    pub curve_json: Option<String>,
}

/// What a finished run saves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FinishedBacktest<'a> {
    pub finished_at: i64,
    pub total_pnl: f64,
    pub max_drawdown: f64,
    /// The serialised trade log.
    pub trades_json: &'a str,
    pub walk_forward_json: Option<&'a str>,
    pub curve_json: Option<&'a str>,
}

impl Store {
    /// Every saved strategy, oldest first.
    pub fn strategies(&self) -> Result<Vec<StrategyRow>, StoreError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT id, name, template, json, created_at FROM strategies ORDER BY id",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(StrategyRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    template: row.get(2)?,
                    json: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Saves a strategy (or renames the one with this id when `id` matches).
    /// Returns the id.
    pub fn save_strategy(
        &self,
        id: Option<i64>,
        name: &str,
        template: &str,
        json: &str,
        now_ms: i64,
    ) -> Result<i64, StoreError> {
        let conn = self.lock()?;
        if let Some(existing) = id {
            conn.execute(
                "UPDATE strategies SET name = ?1, template = ?2, json = ?3 WHERE id = ?4",
                (name, template, json, existing),
            )?;
            Ok(existing)
        } else {
            conn.execute(
                "INSERT INTO strategies (name, template, json, created_at) VALUES (?1, ?2, ?3, ?4)",
                (name, template, json, now_ms),
            )?;
            Ok(conn.last_insert_rowid())
        }
    }

    pub fn delete_strategy(&self, id: i64) -> Result<(), StoreError> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM strategies WHERE id = ?1", (id,))?;
        Ok(())
    }

    /// Starts a run of the strategy on `symbol` (`NSE:RELIANCE` shaped).
    pub fn open_backtest(
        &self,
        strategy_id: i64,
        symbol: &str,
        interval: &str,
        range: &str,
        now_ms: i64,
    ) -> Result<i64, StoreError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO backtests (strategy_id, symbol, interval, range, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            (strategy_id, symbol, interval, range, now_ms),
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// The strategy's newest finished run, if any.
    pub fn latest_backtest(&self, strategy_id: i64) -> Result<Option<BacktestRow>, StoreError> {
        let conn = self.lock()?;
        Ok(conn
            .query_row(
                "SELECT id, strategy_id, symbol, interval, range, started_at, finished_at,
                        total_pnl, max_drawdown, trades_json, walk_forward_json, curve_json
                 FROM backtests
                 WHERE strategy_id = ?1 AND finished_at IS NOT NULL
                 ORDER BY id DESC LIMIT 1",
                (strategy_id,),
                |r| {
                    Ok(BacktestRow {
                        id: r.get(0)?,
                        strategy_id: r.get(1)?,
                        symbol: r.get(2)?,
                        interval: r.get(3)?,
                        range: r.get(4)?,
                        started_at: r.get(5)?,
                        finished_at: r.get(6)?,
                        total_pnl: r.get(7)?,
                        max_drawdown: r.get(8)?,
                        trades_json: r.get(9)?,
                        walk_forward_json: r.get(10)?,
                        curve_json: r.get(11)?,
                    })
                },
            )
            .optional()?)
    }

    /// Finishes a run with its results.
    pub fn finish_backtest(
        &self,
        run_id: i64,
        run: &FinishedBacktest<'_>,
    ) -> Result<(), StoreError> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE backtests
             SET finished_at = ?2, total_pnl = ?3, max_drawdown = ?4, trades_json = ?5,
                 walk_forward_json = ?6, curve_json = ?7
             WHERE id = ?1",
            (
                run_id,
                run.finished_at,
                run.total_pnl,
                run.max_drawdown,
                run.trades_json,
                run.walk_forward_json,
                run.curve_json,
            ),
        )?;
        Ok(())
    }

    /// The paper state for a strategy: `running` and the serialised
    /// position/P&L, or `None` when the strategy has never run.
    pub fn paper_state(&self, strategy_id: i64) -> Result<Option<(bool, String)>, StoreError> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT running, state_json FROM paper_state WHERE strategy_id = ?1",
            (strategy_id,),
            |row| Ok((row.get::<_, i64>(0)? != 0, row.get(1)?)),
        )
        .optional()
        .map_err(StoreError::from)
    }

    /// Saves the paper state; `running` false means stopped-but-resumable.
    pub fn save_paper_state(
        &self,
        strategy_id: i64,
        running: bool,
        state_json: &str,
    ) -> Result<(), StoreError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO paper_state (strategy_id, running, state_json)
             VALUES (?1, ?2, ?3)
             ON CONFLICT (strategy_id)
             DO UPDATE SET running = ?2, state_json = ?3",
            (strategy_id, i64::from(running), state_json),
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    #[test]
    fn strategies_round_trip_and_delete() {
        let store = store();
        let id = store
            .save_strategy(
                None,
                "Golden Cross",
                "MA Crossover",
                r#"{"version":1}"#,
                1_000,
            )
            .unwrap();
        let rows = store.strategies().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Golden Cross");
        assert_eq!(rows[0].template, "MA Crossover");
        // Renaming works.
        store
            .save_strategy(Some(id), "GC v2", "MA Crossover", r#"{"version":1}"#, 1_100)
            .unwrap();
        assert_eq!(store.strategies().unwrap()[0].name, "GC v2");
        store.delete_strategy(id).unwrap();
        assert!(store.strategies().unwrap().is_empty());
    }

    #[test]
    fn backtests_open_and_finish() {
        let store = store();
        let sid = store
            .save_strategy(None, "S", "MA Crossover", "{}", 1_000)
            .unwrap();
        assert!(store.latest_backtest(sid).unwrap().is_none());
        let run = store
            .open_backtest(sid, "NSE:TCS", "1d", "1y", 2_000)
            .unwrap();
        assert!(
            store.latest_backtest(sid).unwrap().is_none(),
            "an unfinished run is not a result"
        );
        store
            .finish_backtest(
                run,
                &FinishedBacktest {
                    finished_at: 3_000,
                    total_pnl: 1_234.5,
                    max_drawdown: 567.8,
                    trades_json: r#"[{"pnl":1}]"#,
                    walk_forward_json: Some("{}"),
                    curve_json: Some("[]"),
                },
            )
            .unwrap();
        let latest = store.latest_backtest(sid).unwrap().unwrap();
        assert_eq!(latest.id, run);
        assert_eq!(
            (
                latest.symbol.as_str(),
                latest.interval.as_str(),
                latest.range.as_str()
            ),
            ("NSE:TCS", "1d", "1y")
        );
        assert_eq!(latest.total_pnl, Some(1_234.5));
        assert_eq!(latest.walk_forward_json.as_deref(), Some("{}"));
        assert_eq!(latest.curve_json.as_deref(), Some("[]"));
        let conn = store.lock().unwrap();
        let (finished, pnl, dd, trades): (i64, f64, f64, String) = conn
            .query_row(
                "SELECT finished_at, total_pnl, max_drawdown, trades_json FROM backtests WHERE id = ?1",
                (run,),
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(finished, 3_000);
        assert_eq!(pnl, 1_234.5);
        assert_eq!(dd, 567.8);
        assert_eq!(trades, r#"[{"pnl":1}]"#);
    }

    #[test]
    fn paper_state_saves_and_overwrites() {
        let store = store();
        let sid = store.save_strategy(None, "S", "RSI", "{}", 1_000).unwrap();
        assert!(store.paper_state(sid).unwrap().is_none());
        store
            .save_paper_state(sid, true, r#"{"day_pnl":10.0}"#)
            .unwrap();
        let (running, json) = store.paper_state(sid).unwrap().unwrap();
        assert!(running);
        assert_eq!(json, r#"{"day_pnl":10.0}"#);
        // Stopping keeps the state but flips the flag.
        store
            .save_paper_state(sid, false, r#"{"day_pnl":10.0}"#)
            .unwrap();
        let (running, _) = store.paper_state(sid).unwrap().unwrap();
        assert!(!running);
    }
}
