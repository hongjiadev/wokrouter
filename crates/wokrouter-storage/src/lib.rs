//! Durable, non-secret storage for WokRouter.

pub mod config;
pub mod state;

pub use config::{AppConfig, ConfigStore, ServerConfig, UiConfig, VersionedConfig};
pub use state::{RequestMetric, StateHealth, StateStore};

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("configuration revision conflict: expected {expected}, found {actual}")]
    RevisionConflict { expected: u64, actual: u64 },
    #[error("invalid configuration: {message}")]
    InvalidConfig { message: String },
    #[error("failed to serialize configuration: {message}")]
    SerializeConfig { message: String },
    #[error("storage I/O error: {source}")]
    Io {
        #[source]
        source: std::io::Error,
    },
    #[error("state database is corrupt: {message}")]
    StateDatabaseCorrupt { message: String },
    #[error("state database error: {source}")]
    StateDatabase {
        #[source]
        source: rusqlite::Error,
    },
}
