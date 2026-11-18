use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("cannot create database directory: {0}")]
    Io(#[from] std::io::Error),
    /// The file was written by a newer Nivesh. Opening it could corrupt data.
    #[error("database schema v{found} is newer than this build supports (v{supported})")]
    SchemaTooNew { found: u32, supported: u32 },
    #[error("the store lock was poisoned by a panic in another thread")]
    Poisoned,
}
