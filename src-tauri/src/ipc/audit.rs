//! Wire types for the AUDIT view.

use serde::Serialize;
use ts_rs::TS;

/// One audit record as the view lists it.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AuditRowDto {
    pub seq: i64,
    /// UTC ms.
    pub at: i64,
    pub kind: String,
    /// The strategy's name, when the record concerns one that still exists.
    pub strategy: Option<String>,
    pub strategy_id: Option<i64>,
    /// The event's details, as recorded (JSON text).
    pub payload: String,
    /// The record's chain hash (hex).
    pub hash: String,
}

/// The chain check's result.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AuditVerifyDto {
    pub intact: bool,
    /// Rows checked, when intact.
    pub rows: Option<u32>,
    /// The first bad row, when broken.
    pub broken_at: Option<i64>,
    pub reason: Option<String>,
}

impl From<nivesh_store::AuditVerification> for AuditVerifyDto {
    fn from(v: nivesh_store::AuditVerification) -> Self {
        match v {
            nivesh_store::AuditVerification::Intact { rows } => Self {
                intact: true,
                rows: Some(u32::try_from(rows).unwrap_or(u32::MAX)),
                broken_at: None,
                reason: None,
            },
            nivesh_store::AuditVerification::Broken { seq, reason } => Self {
                intact: false,
                rows: None,
                broken_at: Some(seq),
                reason: Some(reason),
            },
        }
    }
}
