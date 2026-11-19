//! The gateway's audit sink in the app: events go into the store's
//! hash-chained audit log. Wiring only; the chain lives in `nivesh-store`.

use std::sync::Arc;

use nivesh_exec::{AuditEvent, AuditSink};
use nivesh_store::{AuditEntry, Store};

pub struct StoreAudit(pub Arc<Store>);

impl AuditSink for StoreAudit {
    fn record(&self, at_ms: i64, event: &AuditEvent) -> Result<(), String> {
        let payload = event.payload();
        self.0
            .append_audit(&AuditEntry {
                at_ms,
                kind: event.kind(),
                strategy_id: Some(event.strategy_id()),
                payload: &payload,
            })
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}
