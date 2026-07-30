mod support;

use std::{fs, num::NonZeroU32, path::PathBuf};

use secrecy::{ExposeSecret, SecretString};
use serde_json::json;
use tempfile::tempdir;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path},
};
use wokrouter_wokcore_client::{ManagementError, WokCoreClient};

use support::{INSTANCE_ID, write_discovery, write_discovery_with_pid};

const MANAGEMENT_TOKEN: &str = "wok_proxy_v1_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const CLIENT_TOKEN: &str = "wok_proxy_v1_BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

#[tokio::test]
async fn verified_integration_runtime_comes_only_from_the_handshake() {
    let server = MockServer::start().await;
    mount_handshake(&server, INSTANCE_ID).await;
    let (_fixture, client) = client(&server);

    let runtime = client.integration_runtime().await.unwrap();

    assert_eq!(runtime.base_url(), format!("{}/v1/", server.uri()));
    assert!(runtime.supports_protocol("openai.responses.v1"));
    assert!(runtime.supports_protocol("anthropic.messages.v1"));
    assert!(runtime.supports_capability("client_token.issue"));
    assert!(!runtime.supports_capability("provider.catalog.copy"));
}

#[tokio::test]
async fn runtime_bound_token_issue_revokes_when_discovery_identity_changes() {
    const SECOND_INSTANCE_ID: &str = "11234567-89ab-4cde-8fab-0123456789ab";

    let first = MockServer::start().await;
    let second = MockServer::start().await;
    mount_handshake(&first, INSTANCE_ID).await;
    mount_handshake(&second, SECOND_INSTANCE_ID).await;
    let fixture = tempdir().unwrap();
    let discovery = fixture.path().join("discovery.json");
    write_discovery(&discovery, &first.uri(), INSTANCE_ID, 1, None);
    let client = WokCoreClient::new(&discovery).unwrap();
    let runtime = client.integration_runtime().await.unwrap();
    let replacement_discovery = discovery.clone();
    let second_uri = second.uri();
    Mock::given(method("POST"))
        .and(path("/wokcore/v1/clients/authorize"))
        .respond_with(move |_: &wiremock::Request| {
            write_discovery(
                &replacement_discovery,
                &second_uri,
                SECOND_INSTANCE_ID,
                1,
                None,
            );
            ResponseTemplate::new(201).set_body_json(json!({
                "client_id": "wokrouter.codex",
                "token_id": "01234567-89ab-4cde-8fab-0123456789ab",
                "token": CLIENT_TOKEN,
                "scopes": ["proxy.use"]
            }))
        })
        .expect(1)
        .mount(&first)
        .await;
    Mock::given(method("DELETE"))
        .and(path(
            "/wokcore/v1/clients/wokrouter.codex/tokens/01234567-89ab-4cde-8fab-0123456789ab",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "revoked": true
        })))
        .expect(1)
        .mount(&first)
        .await;

    assert_eq!(
        client
            .issue_proxy_token_for_runtime(
                &runtime,
                &SecretString::from(MANAGEMENT_TOKEN.to_owned()),
                "wokrouter.codex",
            )
            .await
            .unwrap_err(),
        ManagementError::InvalidRuntime
    );
}

#[tokio::test]
async fn bound_runtime_token_issue_rejects_replaced_process_before_token_mutation() {
    let first = MockServer::start().await;
    let second = MockServer::start().await;
    let (fixture, discovery, client, runtime) = bound_client_with_runtime(&first).await;
    mount_handshake(&second, INSTANCE_ID).await;
    mount_token_mutations(&first).await;
    mount_token_mutations(&second).await;
    write_discovery_with_pid(&discovery, &second.uri(), INSTANCE_ID, 42, 1, None);

    assert_eq!(
        client
            .issue_proxy_token_for_runtime(
                &runtime,
                &SecretString::from(MANAGEMENT_TOKEN.to_owned()),
                "wokrouter.codex",
            )
            .await
            .unwrap_err(),
        ManagementError::Missing
    );
    assert_no_token_mutations(&first).await;
    assert_no_token_mutations(&second).await;
    drop(fixture);
}

#[tokio::test]
async fn bound_runtime_token_issue_rejects_missing_discovery_before_token_mutation() {
    let server = MockServer::start().await;
    let (fixture, discovery, client, runtime) = bound_client_with_runtime(&server).await;
    mount_token_mutations(&server).await;
    fs::remove_file(&discovery).unwrap();

    assert_eq!(
        client
            .issue_proxy_token_for_runtime(
                &runtime,
                &SecretString::from(MANAGEMENT_TOKEN.to_owned()),
                "wokrouter.codex",
            )
            .await
            .unwrap_err(),
        ManagementError::Missing
    );
    assert_no_token_mutations(&server).await;
    drop(fixture);
}

#[tokio::test]
async fn bound_runtime_token_issue_rejects_invalid_discovery_before_token_mutation() {
    let server = MockServer::start().await;
    let (fixture, discovery, client, runtime) = bound_client_with_runtime(&server).await;
    mount_token_mutations(&server).await;
    write_discovery_with_pid(&discovery, &server.uri(), INSTANCE_ID, 0, 1, None);

    assert_eq!(
        client
            .issue_proxy_token_for_runtime(
                &runtime,
                &SecretString::from(MANAGEMENT_TOKEN.to_owned()),
                "wokrouter.codex",
            )
            .await
            .unwrap_err(),
        ManagementError::InvalidRuntime
    );
    assert_no_token_mutations(&server).await;
    drop(fixture);
}

#[tokio::test]
async fn bound_runtime_token_issue_with_preallocated_id_rejects_replaced_process_before_token_mutation()
 {
    let first = MockServer::start().await;
    let second = MockServer::start().await;
    let (fixture, discovery, client, runtime) = bound_client_with_runtime(&first).await;
    mount_handshake(&second, INSTANCE_ID).await;
    mount_token_mutations(&first).await;
    mount_token_mutations(&second).await;
    write_discovery_with_pid(&discovery, &second.uri(), INSTANCE_ID, 42, 1, None);

    assert_eq!(
        client
            .issue_proxy_token_for_runtime_with_preallocated_id(
                &runtime,
                &SecretString::from(MANAGEMENT_TOKEN.to_owned()),
                "wokrouter.codex",
                "01234567-89ab-4cde-8fab-0123456789ab",
            )
            .await
            .unwrap_err(),
        ManagementError::Missing
    );
    assert_no_token_mutations(&first).await;
    assert_no_token_mutations(&second).await;
    drop(fixture);
}

#[tokio::test]
async fn proxy_tokens_are_issued_only_through_clients_manage() {
    let server = MockServer::start().await;
    let authority = server.uri().trim_start_matches("http://").to_owned();
    Mock::given(method("POST"))
        .and(path("/wokcore/v1/clients/authorize"))
        .and(header("host", authority.as_str()))
        .and(header(
            "authorization",
            format!("Bearer {MANAGEMENT_TOKEN}").as_str(),
        ))
        .and(body_json(json!({
            "client_id": "wokrouter.codex",
            "scopes": ["proxy.use"]
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "client_id": "wokrouter.codex",
            "token_id": "01234567-89ab-4cde-8fab-0123456789ab",
            "token": CLIENT_TOKEN,
            "scopes": ["proxy.use"],
            "future_field": true
        })))
        .expect(1)
        .mount(&server)
        .await;
    let (_fixture, client) = client(&server);

    let issued = client
        .issue_proxy_token(
            &SecretString::from(MANAGEMENT_TOKEN.to_owned()),
            "wokrouter.codex",
        )
        .await
        .unwrap();

    assert_eq!(issued.client_id(), "wokrouter.codex");
    assert_eq!(issued.token_id(), "01234567-89ab-4cde-8fab-0123456789ab");
    assert_eq!(issued.token().expose_secret(), CLIENT_TOKEN);
    assert_eq!(issued.scopes(), ["proxy.use"]);
    assert!(!format!("{issued:?}").contains(CLIENT_TOKEN));
}

#[tokio::test]
async fn runtime_bound_issue_preallocates_the_recoverable_id_and_inspects_it() {
    let server = MockServer::start().await;
    mount_handshake(&server, INSTANCE_ID).await;
    let token_id = "01234567-89ab-4cde-8fab-0123456789ab";
    Mock::given(method("POST"))
        .and(path("/wokcore/v1/clients/authorize"))
        .and(body_json(json!({
            "client_id": "wokrouter.codex",
            "token_id": token_id,
            "scopes": ["proxy.use"]
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "client_id": "wokrouter.codex",
            "token_id": token_id,
            "token": CLIENT_TOKEN,
            "scopes": ["proxy.use"]
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/wokcore/v1/clients/wokrouter.codex/tokens/{token_id}"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"active": true})))
        .expect(1)
        .mount(&server)
        .await;
    let (_fixture, client) = client(&server);
    let runtime = client.integration_runtime().await.unwrap();
    let management = SecretString::from(MANAGEMENT_TOKEN.to_owned());

    let issued = client
        .issue_proxy_token_for_runtime_with_preallocated_id(
            &runtime,
            &management,
            "wokrouter.codex",
            token_id,
        )
        .await
        .unwrap();

    assert_eq!(issued.token_id(), token_id);
    assert!(
        client
            .client_token_active_for_runtime(&runtime, &management, "wokrouter.codex", token_id)
            .await
            .unwrap()
    );
}

fn client(server: &MockServer) -> (tempfile::TempDir, WokCoreClient) {
    let fixture = tempdir().unwrap();
    let discovery = fixture.path().join("discovery.json");
    write_discovery(&discovery, &server.uri(), INSTANCE_ID, 1, None);
    (
        fixture,
        WokCoreClient::new(discovery).expect("test client should initialize"),
    )
}

async fn bound_client_with_runtime(
    server: &MockServer,
) -> (
    tempfile::TempDir,
    PathBuf,
    WokCoreClient,
    wokrouter_wokcore_client::IntegrationRuntime,
) {
    mount_handshake(server, INSTANCE_ID).await;
    let fixture = tempdir().unwrap();
    let discovery = fixture.path().join("discovery.json");
    write_discovery_with_pid(&discovery, &server.uri(), INSTANCE_ID, 41, 1, None);
    let client = WokCoreClient::new(&discovery)
        .unwrap()
        .bound_to_process(NonZeroU32::new(41).unwrap());
    let runtime = client.integration_runtime().await.unwrap();
    (fixture, discovery, client, runtime)
}

async fn mount_token_mutations(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/wokcore/v1/clients/authorize"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "client_id": "wokrouter.codex",
            "token_id": "01234567-89ab-4cde-8fab-0123456789ab",
            "token": CLIENT_TOKEN,
            "scopes": ["proxy.use"]
        })))
        .mount(server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(
            "/wokcore/v1/clients/wokrouter.codex/tokens/01234567-89ab-4cde-8fab-0123456789ab",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"revoked": true})))
        .mount(server)
        .await;
}

async fn assert_no_token_mutations(server: &MockServer) {
    let requests = server.received_requests().await.unwrap();
    assert!(
        requests.iter().all(|request| {
            !matches!(
                request.url.path(),
                "/wokcore/v1/clients/authorize"
                    | "/wokcore/v1/clients/wokrouter.codex/tokens/01234567-89ab-4cde-8fab-0123456789ab"
            )
        }),
        "bound token issuance must not authorize or revoke after discovery changes"
    );
}

async fn mount_handshake(server: &MockServer, instance_id: &str) {
    let authority = server.uri().trim_start_matches("http://").to_owned();
    Mock::given(method("GET"))
        .and(path("/wokcore/v1/health"))
        .and(header("host", authority.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "ok",
            "instance_id": instance_id
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/wokcore/v1/capabilities"))
        .and(header("host", authority.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "wokcore_version": "0.1.0",
            "management_api_major": 1,
            "minimum_management_api_major": 1,
            "maximum_management_api_major": 1,
            "provider_protocols": [
                "anthropic.messages.v1",
                "openai.chat_completions.v1",
                "openai.responses.v1"
            ],
            "capabilities": [
                "client_token.issue",
                "client_token.inspect",
                "client_token.revoke"
            ],
            "instance_id": instance_id,
            "installation_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        })))
        .mount(server)
        .await;
}
