use std::{
    fs,
    sync::{Arc, Barrier},
    thread,
};

use wokrouter_storage::{RequestMetric, StateStore, StorageError};

#[test]
fn opening_new_database_applies_migrations_and_wal() {
    let directory = tempfile::tempdir().unwrap();
    let store = StateStore::open(directory.path().join("state.db")).unwrap();

    assert_eq!(store.health().unwrap().schema_version, 1);
    assert_eq!(store.pragma_journal_mode().unwrap(), "wal");
    assert_eq!(store.pragma_foreign_keys().unwrap(), 1);
    assert_eq!(store.pragma_busy_timeout().unwrap(), 5_000);
}

#[test]
fn reopening_database_preserves_the_migrated_schema() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");

    StateStore::open(&path).unwrap();
    let reopened = StateStore::open(path).unwrap();

    assert_eq!(reopened.health().unwrap().schema_version, 1);
}

#[test]
fn opening_database_with_empty_migration_ledger_applies_version_one() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(
            "CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);",
        )
        .unwrap();

    let store = StateStore::open(&path).unwrap();

    assert_eq!(store.health().unwrap().schema_version, 1);
    let connection = rusqlite::Connection::open(path).unwrap();
    let table_count = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('accounts', 'quota_windows', 'thread_affinities', 'request_metrics', 'orphan_secrets')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    assert_eq!(table_count, 5);
}

#[test]
fn concurrent_first_opens_complete_the_initial_migration() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    let barrier = Arc::new(Barrier::new(2));

    let handles = (0..2)
        .map(|_| {
            let barrier = Arc::clone(&barrier);
            let path = path.clone();
            thread::spawn(move || {
                barrier.wait();
                StateStore::open(path).unwrap().health().unwrap()
            })
        })
        .collect::<Vec<_>>();

    for handle in handles {
        assert_eq!(handle.join().unwrap().schema_version, 1);
    }
}

#[test]
fn recording_metric_persists_only_request_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    let store = StateStore::open(&path).unwrap();

    store
        .record_request_metric(&RequestMetric {
            request_id: "request-1".to_owned(),
            provider_id: "provider-1".to_owned(),
            model: "model-1".to_owned(),
            started_at: "2026-07-24T00:00:00Z".to_owned(),
            latency_ms: 125,
            input_tokens: Some(10),
            output_tokens: Some(20),
            status_code: 200,
            error_code: None,
        })
        .unwrap();

    let connection = rusqlite::Connection::open(path).unwrap();
    let stored = connection
        .query_row(
            "SELECT provider_id, model, started_at, latency_ms, input_tokens, output_tokens, status_code, error_code FROM request_metrics WHERE request_id = ?1",
            ["request-1"],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            },
        )
        .unwrap();

    assert_eq!(
        stored,
        (
            "provider-1".to_owned(),
            "model-1".to_owned(),
            "2026-07-24T00:00:00Z".to_owned(),
            125,
            Some(10),
            Some(20),
            200,
            None,
        )
    );
}

#[test]
fn opening_invalid_database_returns_corruption_error_without_overwriting_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    let invalid_bytes = b"not a sqlite database";
    fs::write(&path, invalid_bytes).unwrap();

    let error = StateStore::open(&path).unwrap_err();

    assert!(matches!(error, StorageError::StateDatabaseCorrupt { .. }));
    assert_eq!(fs::read(path).unwrap(), invalid_bytes);
}
