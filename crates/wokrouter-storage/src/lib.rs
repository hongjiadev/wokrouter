//! WokRouter preferences and native WokCore client-token storage.

pub mod config;
mod wokcore_token;

pub use config::{AppConfig, ConfigStore, UiConfig, VersionedConfig};
pub use wokcore_token::{NativeWokCoreTokenVault, TokenVaultError, WokCoreTokenVault};

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
}
