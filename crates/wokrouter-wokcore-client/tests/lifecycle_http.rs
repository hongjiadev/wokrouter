mod support;

use secrecy::SecretString;
use serde_json::json;
use tempfile::tempdir;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};
use wokrouter_wokcore_client::{ServiceError, ServicePhase, WokCoreClient};

use support::{INSTANCE_ID, write_discovery};

const TOKEN: &str = "wok_proxy_v1_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

fn client(server: &MockServer) -> (tempfile::TempDir, WokCoreClient) {
    let fixture = tempdir().unwrap();
    let discovery = fixture.path().join("discovery.json");
    write_discovery(&discovery, &server.uri(), INSTANCE_ID, 1, None);
    (
        fixture,
        WokCoreClient::new(discovery).expect("test client should initialize"),
    )
}

#[tokio::test]
async fn service_status_uses_sensitive_bearer_auth_and_accepts_unknown_fields() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wokcore/v1/service/status"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "phase": "running",
            "active_requests": 17,
            "future_status_field": true
        })))
        .expect(1)
        .mount(&server)
        .await;
    let (_fixture, client) = client(&server);

    let status = client
        .service_status(&SecretString::from(TOKEN.to_owned()))
        .await
        .unwrap();

    assert_eq!(status.phase, ServicePhase::Running);
    assert_eq!(status.active_requests, 17);
}

#[tokio::test]
async fn stop_drains_before_requesting_process_stop() {
    let server = MockServer::start().await;
    for (endpoint, phase) in [
        ("/wokcore/v1/service/drain", "draining"),
        ("/wokcore/v1/service/stop", "stopping"),
    ] {
        Mock::given(method("POST"))
            .and(path(endpoint))
            .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "phase": phase,
                "active_requests": 0
            })))
            .expect(1)
            .mount(&server)
            .await;
    }
    let (_fixture, client) = client(&server);

    client
        .stop(&SecretString::from(TOKEN.to_owned()))
        .await
        .unwrap();
}

#[tokio::test]
async fn unauthorized_service_request_returns_only_a_stable_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wokcore/v1/service/status"))
        .respond_with(ResponseTemplate::new(401).set_body_string(TOKEN))
        .mount(&server)
        .await;
    let (_fixture, client) = client(&server);

    let error = client
        .service_status(&SecretString::from(TOKEN.to_owned()))
        .await
        .unwrap_err();

    assert_eq!(error, ServiceError::Unauthorized);
    assert!(!error.to_string().contains(TOKEN));
    assert!(!format!("{error:?}").contains(TOKEN));
}
