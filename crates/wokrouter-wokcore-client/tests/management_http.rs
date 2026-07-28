mod support;

use secrecy::SecretString;
use serde_json::json;
use tempfile::tempdir;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path, query_param},
};
use wokrouter_wokcore_client::{
    DiagnosticExportQuery, DiagnosticLogQuery, ManagementError, ProviderCandidate,
    ProviderCommitRequest, ProviderConfig, ProviderSecretCreate, ProviderSecretPurpose,
    RoutingConfig, SessionQuery, SessionSource, UsageGroup, UsageQuery, WokCoreClient,
};

use support::{INSTANCE_ID, write_discovery};

const TOKEN: &str = "wok_proxy_v1_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const SESSION_KEY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn client(server: &MockServer) -> (tempfile::TempDir, WokCoreClient) {
    let fixture = tempdir().unwrap();
    let discovery = fixture.path().join("discovery.json");
    write_discovery(&discovery, &server.uri(), INSTANCE_ID, 1, None);
    (
        fixture,
        WokCoreClient::new(discovery).expect("test client should initialize"),
    )
}

fn token() -> SecretString {
    SecretString::from(TOKEN.to_owned())
}

#[tokio::test]
async fn sessions_are_bounded_paged_and_forward_compatible() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wokcore/v1/sessions"))
        .and(query_param("source", "codex"))
        .and(query_param("limit", "25"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "schema_version": 1,
            "items": [{
                "session_key": SESSION_KEY,
                "source": "codex",
                "created_at": "2026-07-27T08:00:00Z",
                "last_active_at": "2026-07-27T08:01:00Z",
                "availability": "available",
                "message_count": 2,
                "usage_event_count": 1,
                "title": "Synthetic session",
                "future_session_field": true
            }],
            "next_cursor": null,
            "index_status": {
                "phase": "idle",
                "sources": [{
                    "source": "codex",
                    "status": "available",
                    "future_source_field": true
                }]
            },
            "future_response_field": {"enabled": true}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let (_fixture, client) = client(&server);

    let response = client
        .list_sessions(
            &token(),
            &SessionQuery {
                source: Some(SessionSource::Codex),
                limit: Some(25),
                ..SessionQuery::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(response.items.len(), 1);
    assert_eq!(response.items[0].session_key, SESSION_KEY);
    assert_eq!(response.items[0].message_count, 2);
}

#[tokio::test]
async fn provider_management_uses_typed_revisioned_requests() {
    let server = MockServer::start().await;
    let candidate = ProviderCandidate {
        providers: ProviderConfig::default(),
        routing: RoutingConfig::default(),
    };
    Mock::given(method("POST"))
        .and(path("/wokcore/v1/providers/config/validate"))
        .and(body_json(&candidate))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "schema_version": 1,
            "valid": true,
            "provider_count": 0,
            "models": [],
            "future_validation_field": true
        })))
        .expect(1)
        .mount(&server)
        .await;
    let commit = ProviderCommitRequest {
        expected_revision: 7,
        providers: ProviderConfig::default(),
        routing: RoutingConfig::default(),
    };
    Mock::given(method("PUT"))
        .and(path("/wokcore/v1/providers/config"))
        .and(body_json(&commit))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "schema_version": 1,
            "revision": 8,
            "snapshot_revision": 8,
            "provider_count": 0,
            "models": []
        })))
        .expect(1)
        .mount(&server)
        .await;
    let (_fixture, client) = client(&server);

    assert!(
        client
            .validate_provider_config(&token(), &candidate)
            .await
            .unwrap()
            .valid
    );
    assert_eq!(
        client
            .commit_provider_config(&token(), &commit)
            .await
            .unwrap()
            .revision,
        8
    );
}

#[tokio::test]
async fn provider_secret_material_never_appears_in_errors() {
    let server = MockServer::start().await;
    let secret = "synthetic-provider-secret-that-must-not-leak";
    Mock::given(method("POST"))
        .and(path("/wokcore/v1/provider-secrets"))
        .respond_with(ResponseTemplate::new(500).set_body_string(secret))
        .expect(1)
        .mount(&server)
        .await;
    let (_fixture, client) = client(&server);
    let request = ProviderSecretCreate::new(
        "synthetic",
        None,
        ProviderSecretPurpose::ApiKey,
        SecretString::from(secret.to_owned()),
    );

    let error = client
        .create_provider_secret(&token(), &request)
        .await
        .unwrap_err();

    assert_eq!(error, ManagementError::InvalidResponse);
    assert!(!error.to_string().contains(secret));
    assert!(!format!("{error:?}").contains(secret));
}

#[tokio::test]
async fn usage_and_logs_are_requested_on_demand_with_explicit_pages() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wokcore/v1/usage"))
        .and(query_param("group_by", "source"))
        .and(query_param("limit", "20"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "schema_version": 1,
            "group_by": "source",
            "totals": {
                "input_tokens": 10,
                "output_tokens": 5,
                "cache_read_tokens": 2,
                "cache_write_tokens": 1,
                "reasoning_tokens": 3,
                "session_count": 1
            },
            "buckets": [],
            "next_cursor": null
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/wokcore/v1/logs"))
        .and(query_param("limit", "10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "schema_version": 1,
            "items": [{"level": "info", "message": "synthetic"}],
            "next_cursor": null,
            "dropped_events": 0
        })))
        .expect(1)
        .mount(&server)
        .await;
    let (_fixture, client) = client(&server);

    let usage = client
        .usage(
            &token(),
            &UsageQuery {
                group_by: Some(UsageGroup::Source),
                limit: Some(20),
                ..UsageQuery::default()
            },
        )
        .await
        .unwrap();
    let logs = client
        .diagnostic_logs(
            &token(),
            &DiagnosticLogQuery {
                limit: Some(10),
                ..DiagnosticLogQuery::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(usage.totals.input_tokens, 10);
    assert_eq!(logs.items.len(), 1);
}

#[tokio::test]
async fn diagnostic_export_rejects_the_first_byte_over_the_requested_bound() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wokcore/v1/diagnostics/export"))
        .and(query_param("max_bytes", "65536"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; 65_537]))
        .expect(1)
        .mount(&server)
        .await;
    let (_fixture, client) = client(&server);

    let error = client
        .export_diagnostics(
            &token(),
            &DiagnosticExportQuery {
                max_bytes: Some(65_536),
                ..DiagnosticExportQuery::default()
            },
        )
        .await
        .unwrap_err();

    assert_eq!(error, ManagementError::InvalidResponse);
}
