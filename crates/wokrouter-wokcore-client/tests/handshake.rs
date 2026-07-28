mod support;

use std::net::TcpListener;

use serde_json::json;
use tempfile::tempdir;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};
use wokrouter_wokcore_client::{CoreConnection, ManagementError, WokCoreClient};

use support::{INSTANCE_ID, mount_handshake, write_discovery};

#[tokio::test]
async fn compatible_handshake_accepts_unknown_same_major_fields() {
    let server = MockServer::start().await;
    mount_handshake(&server, INSTANCE_ID).await;
    let fixture = tempdir().unwrap();
    let discovery = fixture.path().join("discovery.json");
    write_discovery(&discovery, &server.uri(), INSTANCE_ID, 1, None);

    let connection = WokCoreClient::new(discovery).unwrap().connection().await;
    let CoreConnection::Running(handshake) = connection else {
        panic!("expected a running handshake");
    };

    assert_eq!(handshake.instance_id, INSTANCE_ID);
    assert_eq!(handshake.version, "0.1.0");
    assert_eq!(handshake.management_api_major, 1);
    assert!(handshake.provider_protocols.contains("openai_responses"));
    assert!(handshake.capabilities.contains("service.status"));
}

#[tokio::test]
async fn legacy_same_major_runtime_without_installation_id_remains_running() {
    let server = MockServer::start().await;
    let authority = server.uri().trim_start_matches("http://").to_owned();
    Mock::given(method("GET"))
        .and(path("/wokcore/v1/health"))
        .and(header("host", authority.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "ok",
            "instance_id": INSTANCE_ID
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/wokcore/v1/capabilities"))
        .and(header("host", authority.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "wokcore_version": "0.1.0",
            "management_api_major": 1,
            "minimum_management_api_major": 1,
            "maximum_management_api_major": 1,
            "provider_protocols": ["openai.responses.v1"],
            "capabilities": ["service.status"],
            "instance_id": INSTANCE_ID
        })))
        .mount(&server)
        .await;
    let fixture = tempdir().unwrap();
    let discovery = fixture.path().join("discovery.json");
    write_discovery(&discovery, &server.uri(), INSTANCE_ID, 1, None);
    let client = WokCoreClient::new(discovery).unwrap();

    let CoreConnection::Running(handshake) = client.connection().await else {
        panic!("expected a running legacy handshake");
    };
    assert_eq!(handshake.installation_id, None);
    assert_eq!(
        client.integration_runtime().await.unwrap_err(),
        ManagementError::Incompatible
    );
}

#[tokio::test]
async fn stale_discovery_with_refused_connection_is_stopped() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let fixture = tempdir().unwrap();
    let discovery = fixture.path().join("discovery.json");
    write_discovery(&discovery, &base_url, INSTANCE_ID, 1, None);

    assert_eq!(
        WokCoreClient::new(discovery).unwrap().connection().await,
        CoreConnection::Stopped
    );
}

#[tokio::test]
async fn identity_mismatch_is_invalid_runtime() {
    let server = MockServer::start().await;
    mount_handshake(&server, "11234567-89ab-4cde-8fab-0123456789ab").await;
    let fixture = tempdir().unwrap();
    let discovery = fixture.path().join("discovery.json");
    write_discovery(&discovery, &server.uri(), INSTANCE_ID, 1, None);

    assert_eq!(
        WokCoreClient::new(discovery).unwrap().connection().await,
        CoreConnection::InvalidRuntime
    );
}

#[tokio::test]
async fn non_overlapping_api_major_is_incompatible_without_http_fallback() {
    let fixture = tempdir().unwrap();
    let discovery = fixture.path().join("discovery.json");
    write_discovery(&discovery, "http://127.0.0.1:8765", INSTANCE_ID, 2, None);

    let connection = WokCoreClient::new(discovery).unwrap().connection().await;
    let CoreConnection::Incompatible(compatibility) = connection else {
        panic!("expected an incompatible connection");
    };
    assert_eq!(compatibility.wokcore_minimum_api_major, 2);
    assert_eq!(compatibility.wokcore_maximum_api_major, 2);
    assert_eq!(compatibility.wokrouter_minimum_api_major, 1);
    assert_eq!(compatibility.wokrouter_maximum_api_major, 1);
}

#[tokio::test]
async fn redirects_are_not_followed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wokcore/v1/health"))
        .respond_with(
            ResponseTemplate::new(307).insert_header("location", "http://127.0.0.1:9/not-followed"),
        )
        .mount(&server)
        .await;
    let fixture = tempdir().unwrap();
    let discovery = fixture.path().join("discovery.json");
    write_discovery(&discovery, &server.uri(), INSTANCE_ID, 1, None);

    assert_eq!(
        WokCoreClient::new(discovery).unwrap().connection().await,
        CoreConnection::InvalidRuntime
    );
}
