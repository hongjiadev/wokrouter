mod support;

use std::fs;

use serde_json::json;
use tempfile::tempdir;
use wokrouter_wokcore_client::{CoreConnection, WokCoreClient};

use support::{INSTANCE_ID, write_discovery};

#[tokio::test]
async fn absent_discovery_is_missing() {
    let fixture = tempdir().unwrap();
    let client = WokCoreClient::new(fixture.path().join("discovery.json")).unwrap();

    assert_eq!(client.connection().await, CoreConnection::Missing);
}

#[tokio::test]
async fn oversized_discovery_is_invalid_runtime() {
    let fixture = tempdir().unwrap();
    let path = fixture.path().join("discovery.json");
    fs::write(&path, vec![b'x'; 16 * 1024 + 1]).unwrap();
    let client = WokCoreClient::new(path).unwrap();

    assert_eq!(client.connection().await, CoreConnection::InvalidRuntime);
}

#[tokio::test]
async fn discovery_rejects_unsafe_base_urls_and_invalid_identifiers() {
    let invalid_records = [
        ("http://localhost:8765", INSTANCE_ID, 1),
        ("http://user@127.0.0.1:8765", INSTANCE_ID, 1),
        ("http://127.0.0.1:0", INSTANCE_ID, 1),
        ("http://127.0.0.1:8765/path", INSTANCE_ID, 1),
        ("http://127.0.0.1:8765?query=1", INSTANCE_ID, 1),
        ("http://127.0.0.1:8765#fragment", INSTANCE_ID, 1),
        ("http://127.0.0.1:8765", "not-a-uuid", 1),
        ("http://127.0.0.1:8765", INSTANCE_ID, 0),
    ];

    for (base_url, instance_id, api_major) in invalid_records {
        let fixture = tempdir().unwrap();
        let path = fixture.path().join("discovery.json");
        write_discovery(&path, base_url, instance_id, api_major, None);
        let client = WokCoreClient::new(path).unwrap();

        assert_eq!(
            client.connection().await,
            CoreConnection::InvalidRuntime,
            "{base_url}"
        );
    }
}

#[tokio::test]
async fn discovery_rejects_unknown_fields() {
    let fixture = tempdir().unwrap();
    let path = fixture.path().join("discovery.json");
    write_discovery(
        &path,
        "http://127.0.0.1:8765",
        INSTANCE_ID,
        1,
        Some(("unexpected", json!(true))),
    );
    let client = WokCoreClient::new(path).unwrap();

    assert_eq!(client.connection().await, CoreConnection::InvalidRuntime);
}
