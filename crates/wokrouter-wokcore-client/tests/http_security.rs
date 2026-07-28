mod support;

use serde_json::json;
use tempfile::tempdir;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};
use wokrouter_wokcore_client::{CoreConnection, WokCoreClient};

use support::{INSTANCE_ID, write_discovery};

#[tokio::test]
async fn oversized_public_response_is_rejected_without_retaining_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wokcore/v1/health"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; 64 * 1024 + 1]))
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

#[tokio::test]
async fn incompatible_capability_range_is_reported_explicitly() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wokcore/v1/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "ok",
            "instance_id": INSTANCE_ID
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/wokcore/v1/capabilities"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "wokcore_version": "0.1.0",
            "management_api_major": 2,
            "minimum_management_api_major": 2,
            "maximum_management_api_major": 3,
            "provider_protocols": [],
            "capabilities": [],
            "instance_id": INSTANCE_ID,
            "installation_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        })))
        .mount(&server)
        .await;
    let fixture = tempdir().unwrap();
    let discovery = fixture.path().join("discovery.json");
    write_discovery(&discovery, &server.uri(), INSTANCE_ID, 1, None);

    let connection = WokCoreClient::new(discovery).unwrap().connection().await;
    let CoreConnection::Incompatible(compatibility) = connection else {
        panic!("expected an incompatible connection");
    };
    assert_eq!(compatibility.wokcore_minimum_api_major, 2);
    assert_eq!(compatibility.wokcore_maximum_api_major, 3);
}
