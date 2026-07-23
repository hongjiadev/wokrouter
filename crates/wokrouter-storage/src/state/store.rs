use std::path::Path;

use rusqlite::{Connection, ErrorCode, OptionalExtension, params};

use crate::StorageError;

const INITIAL_MIGRATION: &str = include_str!("../../migrations/0001_initial.sql");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestMetric {
    pub request_id: String,
    pub provider_id: String,
    pub model: String,
    pub started_at: String,
    pub latency_ms: i64,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub status_code: i64,
    pub error_code: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateHealth {
    pub schema_version: i64,
}

#[derive(Debug)]
pub struct StateStore {
    connection: Connection,
}

impl StateStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let mut connection = Connection::open(path).map_err(map_database_error)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;",
            )
            .map_err(map_database_error)?;

        let transaction = connection.transaction().map_err(map_database_error)?;
        let has_migrations = transaction
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
                [],
                |_| Ok(()),
            )
            .optional()
            .map_err(map_database_error)?
            .is_some();
        if !has_migrations {
            transaction
                .execute_batch(INITIAL_MIGRATION)
                .map_err(map_database_error)?;
        }
        transaction.commit().map_err(map_database_error)?;

        Ok(Self { connection })
    }

    pub fn health(&self) -> Result<StateHealth, StorageError> {
        let schema_version = self
            .connection
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get::<_, Option<i64>>(0)
            })
            .map_err(map_database_error)?
            .unwrap_or_default();
        Ok(StateHealth { schema_version })
    }

    pub fn record_request_metric(&self, metric: &RequestMetric) -> Result<(), StorageError> {
        self.connection
            .execute(
                "INSERT INTO request_metrics (request_id, provider_id, model, started_at, latency_ms, input_tokens, output_tokens, status_code, error_code) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    metric.request_id,
                    metric.provider_id,
                    metric.model,
                    metric.started_at,
                    metric.latency_ms,
                    metric.input_tokens,
                    metric.output_tokens,
                    metric.status_code,
                    metric.error_code,
                ],
            )
            .map_err(map_database_error)?;
        Ok(())
    }

    pub fn pragma_journal_mode(&self) -> Result<String, StorageError> {
        self.pragma_value("journal_mode")
    }

    pub fn pragma_foreign_keys(&self) -> Result<i64, StorageError> {
        self.pragma_value("foreign_keys")
    }

    pub fn pragma_busy_timeout(&self) -> Result<i64, StorageError> {
        self.pragma_value("busy_timeout")
    }

    fn pragma_value<T>(&self, name: &str) -> Result<T, StorageError>
    where
        T: rusqlite::types::FromSql,
    {
        self.connection
            .query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
            .map_err(map_database_error)
    }
}

fn map_database_error(error: rusqlite::Error) -> StorageError {
    match &error {
        rusqlite::Error::SqliteFailure(sqlite_error, _)
            if matches!(
                sqlite_error.code,
                ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase
            ) =>
        {
            StorageError::StateDatabaseCorrupt {
                message: error.to_string(),
            }
        }
        _ => StorageError::StateDatabase { source: error },
    }
}
