//! The audit log: an append-only, SHA-256 hash-chained record of every
//! trading decision (PLAN.md §11.9).
//!
//! Each row's hash covers its own fields and the previous row's hash, so
//! editing a row, deleting one, or reordering them breaks the chain from
//! that point on. The database also refuses `UPDATE` and `DELETE` on the
//! table (triggers), and the newest sequence and hash are kept in a
//! separate head row written in the same transaction, so cutting rows off
//! the end is detected too.
//!
//! Appends run with `synchronous = FULL`, so a row that `append_audit`
//! returned is on disk before the caller acts on it (an order is only sent
//! after its audit row is written).
//!
//! Entries never hold secrets: the callers write decisions, orders and
//! fills, never tokens or keys.

use rusqlite::{OptionalExtension as _, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::Store;
use crate::error::StoreError;

/// The previous-hash of the first row.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// What is being recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry<'a> {
    /// UTC ms the event happened.
    pub at_ms: i64,
    /// A short event kind, e.g. `intent`, `risk.approved`, `fill`.
    pub kind: &'a str,
    /// The strategy it concerns, if any.
    pub strategy_id: Option<i64>,
    /// The event's details, as JSON text.
    pub payload: &'a str,
}

/// A stored row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRecord {
    pub seq: i64,
    pub at_ms: i64,
    pub kind: String,
    pub strategy_id: Option<i64>,
    pub payload: String,
    pub prev_hash: String,
    pub hash: String,
}

/// The result of checking the chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditVerification {
    /// Every row links to the one before and hashes to what it says.
    Intact { rows: usize },
    /// The chain is broken at `seq` (the first bad row, or where rows are
    /// missing).
    Broken { seq: i64, reason: String },
}

impl AuditVerification {
    pub fn is_intact(&self) -> bool {
        matches!(self, Self::Intact { .. })
    }
}

/// A filter for reading the log back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuditQuery {
    pub kind_prefix: Option<String>,
    pub strategy_id: Option<i64>,
    /// Only rows at or after this UTC ms.
    pub since_ms: Option<i64>,
    /// At most this many rows, newest first; `None` for all.
    pub limit: Option<usize>,
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
    out
}

/// The hash of a row: SHA-256 over its fields and the previous hash, each
/// field length-prefixed so no two different rows serialise the same way.
pub fn row_hash(
    prev_hash: &str,
    seq: i64,
    at_ms: i64,
    kind: &str,
    strategy_id: Option<i64>,
    payload: &str,
) -> String {
    let mut h = Sha256::new();
    let strategy = strategy_id.map_or_else(String::new, |id| id.to_string());
    for field in [
        prev_hash,
        &seq.to_string(),
        &at_ms.to_string(),
        kind,
        &strategy,
        payload,
    ] {
        h.update((field.len() as u64).to_be_bytes());
        h.update(field.as_bytes());
    }
    hex(&h.finalize())
}

/// Checks a chain of records, oldest first, against the expected head
/// (`None` to skip the truncation check).
pub fn verify_records(records: &[AuditRecord], head: Option<(i64, &str)>) -> AuditVerification {
    let mut prev = GENESIS_HASH.to_owned();
    let mut expected_seq = 1;
    for r in records {
        if r.seq != expected_seq {
            return AuditVerification::Broken {
                seq: expected_seq,
                reason: format!("row {expected_seq} is missing (found {} next)", r.seq),
            };
        }
        if r.prev_hash != prev {
            return AuditVerification::Broken {
                seq: r.seq,
                reason: "does not link to the row before it".into(),
            };
        }
        let hash = row_hash(
            &r.prev_hash,
            r.seq,
            r.at_ms,
            &r.kind,
            r.strategy_id,
            &r.payload,
        );
        if hash != r.hash {
            return AuditVerification::Broken {
                seq: r.seq,
                reason: "its contents do not match its hash (edited)".into(),
            };
        }
        prev = hash;
        expected_seq += 1;
    }
    if let Some((head_seq, head_hash)) = head {
        let last_seq = expected_seq - 1;
        if head_seq != last_seq || (head_seq > 0 && head_hash != prev) {
            return AuditVerification::Broken {
                seq: last_seq + 1,
                reason: format!("the log ends at row {last_seq} but its head says {head_seq}"),
            };
        }
    }
    AuditVerification::Intact {
        rows: records.len(),
    }
}

fn record(r: &rusqlite::Row<'_>) -> rusqlite::Result<AuditRecord> {
    Ok(AuditRecord {
        seq: r.get(0)?,
        at_ms: r.get(1)?,
        kind: r.get(2)?,
        strategy_id: r.get(3)?,
        payload: r.get(4)?,
        prev_hash: r.get(5)?,
        hash: r.get(6)?,
    })
}

/// The head row: the newest sequence number and its hash.
type Head = (i64, String);

const COLUMNS: &str = "seq, at_ms, kind, strategy_id, payload, prev_hash, hash";

impl Store {
    /// Appends an entry and returns the stored row. Durable on return.
    pub fn append_audit(&self, entry: &AuditEntry<'_>) -> Result<AuditRecord, StoreError> {
        let mut conn = self.lock()?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        let result = (|| {
            // IMMEDIATE: no other writer can slip a row in between reading
            // the head and writing the next row.
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let (last_seq, prev_hash): (i64, String) = tx
                .query_row("SELECT seq, hash FROM audit_head WHERE id = 1", [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .optional()?
                .unwrap_or((0, GENESIS_HASH.to_owned()));
            let seq = last_seq + 1;
            let hash = row_hash(
                &prev_hash,
                seq,
                entry.at_ms,
                entry.kind,
                entry.strategy_id,
                entry.payload,
            );
            tx.execute(
                "INSERT INTO audit_log (seq, at_ms, kind, strategy_id, payload, prev_hash, hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                (
                    seq,
                    entry.at_ms,
                    entry.kind,
                    entry.strategy_id,
                    entry.payload,
                    &prev_hash,
                    &hash,
                ),
            )?;
            tx.execute(
                "INSERT INTO audit_head (id, seq, hash) VALUES (1, ?1, ?2)
                 ON CONFLICT (id) DO UPDATE SET seq = excluded.seq, hash = excluded.hash",
                (seq, &hash),
            )?;
            tx.commit()?;
            Ok::<_, StoreError>(AuditRecord {
                seq,
                at_ms: entry.at_ms,
                kind: entry.kind.to_owned(),
                strategy_id: entry.strategy_id,
                payload: entry.payload.to_owned(),
                prev_hash,
                hash,
            })
        })();
        // Back to the store's usual durability for everything else.
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        result
    }

    /// Rows matching `query`, newest first.
    pub fn audit(&self, query: &AuditQuery) -> Result<Vec<AuditRecord>, StoreError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM audit_log
             WHERE (?1 IS NULL OR kind LIKE ?1 || '%')
               AND (?2 IS NULL OR strategy_id = ?2)
               AND (?3 IS NULL OR at_ms >= ?3)
             ORDER BY seq DESC
             LIMIT ?4"
        ))?;
        let limit = query
            .limit
            .map_or(-1, |l| i64::try_from(l).unwrap_or(i64::MAX));
        let rows = stmt
            .query_map(
                (
                    query.kind_prefix.as_deref(),
                    query.strategy_id,
                    query.since_ms,
                    limit,
                ),
                record,
            )?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// Every row, oldest first.
    fn audit_chain(&self) -> Result<(Vec<AuditRecord>, Option<Head>), StoreError> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM audit_log ORDER BY seq"))?;
        let rows = stmt.query_map([], record)?.collect::<Result<_, _>>()?;
        let head = conn
            .query_row("SELECT seq, hash FROM audit_head WHERE id = 1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        Ok((rows, head))
    }

    /// Checks the whole chain, and that nothing was cut off its end.
    pub fn verify_audit(&self) -> Result<AuditVerification, StoreError> {
        let (rows, head) = self.audit_chain()?;
        let head = head.unwrap_or((0, GENESIS_HASH.to_owned()));
        Ok(verify_records(&rows, Some((head.0, &head.1))))
    }

    /// The whole log as JSON Lines, oldest first: one record per line.
    pub fn export_audit_jsonl(&self) -> Result<String, StoreError> {
        let (rows, _) = self.audit_chain()?;
        let mut out = String::new();
        for r in rows {
            let line = serde_json::to_string(&r)
                .map_err(|e| StoreError::Corrupt(format!("cannot encode audit row: {e}")))?;
            out.push_str(&line);
            out.push('\n');
        }
        Ok(out)
    }

    /// The whole log as CSV (RFC 4180 quoting), oldest first.
    pub fn export_audit_csv(&self) -> Result<String, StoreError> {
        let (rows, _) = self.audit_chain()?;
        let mut out = String::from("seq,at_ms,kind,strategy_id,payload,prev_hash,hash\n");
        for r in rows {
            let strategy = r.strategy_id.map_or_else(String::new, |id| id.to_string());
            let fields = [
                r.seq.to_string(),
                r.at_ms.to_string(),
                r.kind,
                strategy,
                r.payload,
                r.prev_hash,
                r.hash,
            ];
            let line: Vec<String> = fields.iter().map(|f| csv_field(f)).collect();
            out.push_str(&line.join(","));
            out.push('\n');
        }
        Ok(out)
    }
}

fn csv_field(f: &str) -> String {
    if f.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", f.replace('"', "\"\""))
    } else {
        f.to_owned()
    }
}

/// Parses a JSONL export back, for verifying an exported copy.
pub fn parse_audit_jsonl(text: &str) -> Result<Vec<AuditRecord>, StoreError> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            serde_json::from_str(l).map_err(|e| StoreError::Corrupt(format!("bad audit line: {e}")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with(n: i64) -> Store {
        let store = Store::open_in_memory().unwrap();
        for i in 1..=n {
            store
                .append_audit(&AuditEntry {
                    at_ms: 1_000 * i,
                    kind: if i % 2 == 0 { "fill" } else { "intent" },
                    strategy_id: Some(7),
                    payload: &format!(r#"{{"n":{i},"note":"a, \"quoted\" value"}}"#),
                })
                .unwrap();
        }
        store
    }

    /// What an attacker with the database file could do: drop the guards
    /// and change rows directly.
    fn tamper(store: &Store, sql: &str) {
        let conn = store.connection().unwrap();
        conn.execute_batch("DROP TRIGGER audit_log_no_update; DROP TRIGGER audit_log_no_delete;")
            .unwrap();
        conn.execute_batch(sql).unwrap();
    }

    #[test]
    fn rows_chain_from_the_genesis_hash() {
        let store = store_with(3);
        let rows = store.audit(&AuditQuery::default()).unwrap();
        assert_eq!(rows.len(), 3);
        let (first, last) = (&rows[2], &rows[0]);
        assert_eq!(first.seq, 1);
        assert_eq!(first.prev_hash, GENESIS_HASH);
        assert_eq!(rows[1].prev_hash, first.hash);
        assert_eq!(last.prev_hash, rows[1].hash);
        assert_eq!(first.hash.len(), 64);
        assert_eq!(
            store.verify_audit().unwrap(),
            AuditVerification::Intact { rows: 3 }
        );
    }

    #[test]
    fn the_database_refuses_edits_and_deletes() {
        let store = store_with(2);
        let conn = store.connection().unwrap();
        assert!(
            conn.execute("UPDATE audit_log SET payload = '{}' WHERE seq = 1", [])
                .is_err()
        );
        assert!(
            conn.execute("DELETE FROM audit_log WHERE seq = 2", [])
                .is_err()
        );
    }

    #[test]
    fn verification_catches_an_edited_row() {
        let store = store_with(4);
        tamper(
            &store,
            "UPDATE audit_log SET payload = '{\"n\":999}' WHERE seq = 2",
        );
        let v = store.verify_audit().unwrap();
        assert!(
            matches!(v, AuditVerification::Broken { seq: 2, .. }),
            "{v:?}"
        );
    }

    #[test]
    fn verification_catches_a_deleted_row() {
        let store = store_with(4);
        tamper(&store, "DELETE FROM audit_log WHERE seq = 2");
        let v = store.verify_audit().unwrap();
        assert!(
            matches!(v, AuditVerification::Broken { seq: 2, .. }),
            "{v:?}"
        );
    }

    #[test]
    fn verification_catches_a_cut_off_tail() {
        let store = store_with(4);
        tamper(&store, "DELETE FROM audit_log WHERE seq = 4");
        let v = store.verify_audit().unwrap();
        assert!(
            matches!(v, AuditVerification::Broken { seq: 4, .. }),
            "{v:?}"
        );
    }

    #[test]
    fn verification_catches_a_rehashed_forgery_that_skips_the_chain() {
        // Editing a row and fixing its own hash still breaks the next link.
        let store = store_with(3);
        let rows = store.audit(&AuditQuery::default()).unwrap();
        let r2 = rows.iter().find(|r| r.seq == 2).unwrap();
        let forged = row_hash(&r2.prev_hash, 2, r2.at_ms, &r2.kind, r2.strategy_id, "{}");
        tamper(
            &store,
            &format!("UPDATE audit_log SET payload = '{{}}', hash = '{forged}' WHERE seq = 2"),
        );
        let v = store.verify_audit().unwrap();
        assert!(
            matches!(v, AuditVerification::Broken { seq: 3, .. }),
            "{v:?}"
        );
    }

    #[test]
    fn exports_round_trip_and_verify() {
        let store = store_with(5);
        let jsonl = store.export_audit_jsonl().unwrap();
        assert_eq!(jsonl.lines().count(), 5);
        let parsed = parse_audit_jsonl(&jsonl).unwrap();
        assert!(verify_records(&parsed, None).is_intact());
        let mut oldest_first = store.audit(&AuditQuery::default()).unwrap();
        oldest_first.reverse();
        assert_eq!(parsed, oldest_first);

        let csv = store.export_audit_csv().unwrap();
        let mut lines = csv.lines();
        assert_eq!(
            lines.next(),
            Some("seq,at_ms,kind,strategy_id,payload,prev_hash,hash")
        );
        let first = lines.next().unwrap();
        assert!(
            first.starts_with(r#"1,1000,intent,7,"{""n"":1,""note"":""a, \""quoted\"" value""}","#),
            "{first}"
        );
    }

    #[test]
    fn queries_filter_by_kind_strategy_and_time() {
        let store = store_with(6);
        let fills = store
            .audit(&AuditQuery {
                kind_prefix: Some("fill".into()),
                ..AuditQuery::default()
            })
            .unwrap();
        assert_eq!(
            fills.iter().map(|r| r.seq).collect::<Vec<_>>(),
            vec![6, 4, 2]
        );
        let recent = store
            .audit(&AuditQuery {
                since_ms: Some(5_000),
                limit: Some(1),
                ..AuditQuery::default()
            })
            .unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].seq, 6);
        let other = store
            .audit(&AuditQuery {
                strategy_id: Some(8),
                ..AuditQuery::default()
            })
            .unwrap();
        assert!(other.is_empty());
    }

    #[test]
    fn an_empty_log_is_intact() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(
            store.verify_audit().unwrap(),
            AuditVerification::Intact { rows: 0 }
        );
    }
}
