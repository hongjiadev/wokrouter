use std::{
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use futures::stream;
use secrecy::SecretString;
use serde_json::Value;
use tempfile::TempDir;
use tokio::time::timeout;
use tower::ServiceExt;
use wokrouter_core::secret::SecretRef;
use wokrouter_daemon::data_plane::{
    CanonicalStream, DataPlaneState, ExecutionContext, FrontDoorMetric, ImmutableSnapshot,
    ListenerSecurity, MetricsSink, RequestLimits, TlsConfig, UpstreamExecutor, build_data_plane,
};
use wokrouter_protocols::canonical::{CanonicalRequest, GatewayError};

const TEST_TIMEOUT: Duration = Duration::from_secs(2);
const CERTIFICATE_PEM: &str = "-----BEGIN CERTIFICATE-----\n\
MIICqTCCAZGgAwIBAgIJALmRPOiyvjDbMA0GCSqGSIb3DQEBCwUAMBQxEjAQBgNVBAMTCWxvY2Fs\n\
aG9zdDAeFw0yNjA3MjMwNTIxNDhaFw0yNjA4MjMwNTIxNDhaMBQxEjAQBgNVBAMTCWxvY2FsaG9z\n\
dDCCASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEBALpYRfW3zsGJmGUSVFVIM5FTrqT0J3bo\n\
cW5vmXzx6RKre1IKdcECBCHyvwFPyz8TPYUlyXNIr2NUPq2uahA6+ql0WXwPksJWe8cekUCMRYcH\n\
m+F+g/LqXJSPndPXnDmhMd1fepkQz50jt4vi6BFk4OVFlGhDGiaeKhqq35pG/skXQ/ggfVY91LcM\n\
QuvU57gLluMCVLhuVsFeFarwCIVXZ6KQzHKPhtHlMAQCI5cKPbXHrBg2gHz5Hn2Rcqyhjn1LAa3z\n\
dRYW/vAbeJpPps/3bhQccDT+oK5Z5BPiNlmiNx2vpa33FbyQCYNAfK5rd3kH0TZDkuiHlP+pwUA+\n\
N84A9s0CAwEAATANBgkqhkiG9w0BAQsFAAOCAQEAiblrB77eP0BZOkDHgZTE9NzFBoQIp7gTA2HG\n\
8WRpxZJ2nnTgCGubbF0GQkK5T/yKSzj/XoKU5SEtCuotsdNn0a6OS5bOHfmpDIHwDdfX/rUl0gBT\n\
WlYPPzTvvIx2F6FDY+f4v4bRyJunmtYvoQpm6PSkUq+N04d4T2ivWzM7Q4WVhAR9bnwMmYwQ9utM\n\
xRBH2Nqs9VZTcbnoDpeiSm87EauBptmGhsBnYYXDZEgXd13n6GAkrpMZRyeSG+FvUAQU/kgSyntZ\n\
cu8P9HQowjI87Hzd4tLos4dRuvbECNxBnSjiWeGvtD876iE1tcqC7VTczYFJYGhMmd91r7QDSgsr\n\
Jw==\n\
-----END CERTIFICATE-----\n";
const PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
MIIEvwIBADANBgkqhkiG9w0BAQEFAASCBKkwggSlAgEAAoIBAQC6WEX1t87BiZhlElRVSDORU66k\n\
9Cd26HFub5l88ekSq3tSCnXBAgQh8r8BT8s/Ez2FJclzSK9jVD6trmoQOvqpdFl8D5LCVnvHHpFA\n\
jEWHB5vhfoPy6lyUj53T15w5oTHdX3qZEM+dI7eL4ugRZODlRZRoQxomnioaqt+aRv7JF0P4IH1W\n\
PdS3DELr1Oe4C5bjAlS4blbBXhWq8AiFV2eikMxyj4bR5TAEAiOXCj21x6wYNoB8+R59kXKsoY59\n\
SwGt83UWFv7wG3iaT6bP924UHHA0/qCuWeQT4jZZojcdr6Wt9xW8kAmDQHyua3d5B9E2Q5Loh5T/\n\
qcFAPjfOAPbNAgMBAAECggEBAIZFiwuWSXXtZpEVlwzofLfv+3zCrRkiPnHcGlYMnewlAjRIczcC\n\
8+VeW8FfNM2bWI3zf2gBbNd+4bcWYTiWtv2ZZ81cD1zXIlOFNBa1vHeixPDDz+Iee11U6t21k812\n\
2E5yOQ3ILkFFdkFm29+EuASckWZbS6GeACq9C2fIVlignqitlo5Ss1D/Zpt1Xyn8ip5Y5osqTCdt\n\
pS3UyGkHLdAlCZVk7lQ2RXxkEUD9uEDHk9ddL8sZasp5x05DFHtygDbvQBlICop9It1Z2MjdqWz1\n\
tpzkTKbdozMglpNKr5aTIiv/fdq9QxcrgnCzYeLZT0uPaqAcsly8hEdv7Euw/wUCgYEAzSJG0BQx\n\
38l68Bp9z9PZlqqiAheAgQNwcwoPex3Y/K6wrYD+7azvdpqv5cVOTHMLcDX0KbgECH0izaFsyW4u\n\
CrTI9qZ0E7iyxs8xHDXTkxzeJ/i4C6zXW/w3MjjEJoKwDfwck9fgFIaoIPug2UT4WiFqZikMYB2t\n\
I1ZOoZpTNF8CgYEA6I1IRATXW58M2FnInkzFCgWrhlF+pY4raSKdZQYL7N9mnTWQbR5ziHdb3wrG\n\
ZJjW3qnCqoNE13FjaHDz0jPK1OkGX31Yd3DaQObuWOMKWPhdXROFPGyn9r+71X3OHd6URc2W81r8\n\
VIaNmtj2WiVRxQr019AchlOq6mJIbc+6hFMCgYAn9hhzaqu4m8huQ8rklLGbr0v2OlvXRjM+xeP0\n\
KQFfYCyc8Dc5V+oiYcoIaeJx9CtzRZ92DRoECVShWGQX7XXcshFAM3cDbISvRCeeBkJcM1B32vUH\n\
mTku+zhJeVOE6Qqg/s8WYgSOGxlfq4VPLidvb3kJw89cXgufia2xv52b4wKBgQDMfM964Dbu3eDR\n\
rcF3UVJCCdJV/fs7YNRTUpjMaJC77YWx35PsH8a/zRT//92MP8lRaj0+6sbyG0aqZAhCYkCND4sH\n\
FJViEd4ZC/eyOZKzwVF3st6Jz5Gyq85jYIiKQ1pmNu3xd6RCPz7tVrLpeb95YLUDwIAUriWwjFPc\n\
G7mK+QKBgQCe8ijUmMQzbqgZgbSFeLFuT5V/bSzxtFRlqRXzMBwg4UhdCrRmsmiNx7eJyuSZxEz0\n\
guBdmDdx/OFM6vm18JSQ+CvukeskvcxNDugF4mg1AynXMWuvIm2ikV9GmiWaxsTjiqEd7j/g5CO6\n\
zype9NmQp6uUIcsTIjnnm2xbqlkOtw==\n\
-----END PRIVATE KEY-----\n";

#[derive(Clone, Default)]
struct RecordingExecutor {
    calls: Arc<AtomicUsize>,
}

impl RecordingExecutor {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl UpstreamExecutor for RecordingExecutor {
    async fn execute(
        &self,
        _context: ExecutionContext,
        _request: CanonicalRequest,
    ) -> Result<CanonicalStream, GatewayError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Box::pin(stream::empty()))
    }
}

struct TestSnapshot {
    reads: AtomicUsize,
}

impl TestSnapshot {
    fn new() -> Self {
        Self {
            reads: AtomicUsize::new(0),
        }
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

impl ImmutableSnapshot for TestSnapshot {
    fn revision(&self) -> u64 {
        self.reads.fetch_add(1, Ordering::SeqCst);
        17
    }
}

#[derive(Default)]
struct RecordingMetrics {
    entries: Mutex<Vec<FrontDoorMetric>>,
}

impl RecordingMetrics {
    fn entries(&self) -> Vec<FrontDoorMetric> {
        self.entries.lock().unwrap().clone()
    }
}

impl MetricsSink for RecordingMetrics {
    fn record(&self, metric: FrontDoorMetric) {
        self.entries.lock().unwrap().push(metric);
    }
}

struct TestApp {
    app: Router,
    executor: RecordingExecutor,
    snapshot: Arc<TestSnapshot>,
    metrics: Arc<RecordingMetrics>,
}

fn test_app(body_limit: usize, bearer: Option<&str>) -> TestApp {
    let executor = RecordingExecutor::default();
    let snapshot = Arc::new(TestSnapshot::new());
    let metrics = Arc::new(RecordingMetrics::default());
    let mut state = DataPlaneState::new(
        Arc::new(executor.clone()),
        RequestLimits {
            json_body_bytes: body_limit,
        },
        snapshot.clone(),
        metrics.clone(),
    );
    if let Some(bearer) = bearer {
        state = state.with_lan_bearer(SecretString::from(bearer.to_owned()));
    }

    TestApp {
        app: build_data_plane(state),
        executor,
        snapshot,
        metrics,
    }
}

#[test]
fn default_json_body_limit_is_16_mib() {
    assert_eq!(RequestLimits::default().json_body_bytes, 16 * 1024 * 1024);
}

async fn send(app: &Router, request: Request<Body>) -> axum::response::Response {
    timeout(TEST_TIMEOUT, app.clone().oneshot(request))
        .await
        .expect("front-door request timed out")
        .expect("front-door service failed")
}

fn request(method: Method, path: &str, body: impl Into<Body>) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .body(body.into())
        .unwrap()
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn registry_exposes_health_and_all_frozen_v1_paths() {
    let test = test_app(1024, None);
    let routes = [
        (Method::GET, "/healthz", None),
        (Method::POST, "/v1/responses", Some("{}")),
        (Method::POST, "/v1/chat/completions", Some("{}")),
        (Method::POST, "/v1/messages", Some("{}")),
        (Method::POST, "/v1/messages/count_tokens", Some("{}")),
        (Method::GET, "/v1/models", None),
        (Method::POST, "/v1/images/generations", Some("{}")),
        (Method::POST, "/v1/images/edits", Some("{}")),
    ];

    for (method, path, body) in routes {
        let response = send(&test.app, request(method, path, body.unwrap_or_default())).await;
        if path == "/healthz" {
            assert_eq!(response.status(), StatusCode::OK, "{path}");
        } else {
            assert_eq!(
                response.status(),
                StatusCode::UNPROCESSABLE_ENTITY,
                "{path}"
            );
        }
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/json",
            "{path}"
        );
        assert!(response.headers().contains_key("x-request-id"), "{path}");
    }

    assert_eq!(test.executor.calls(), 0);
    assert_eq!(test.snapshot.reads(), 8);
    let metrics = test.metrics.entries();
    assert_eq!(metrics.len(), 8);
    assert!(metrics.iter().all(|metric| metric.snapshot_revision == 17));
}

#[tokio::test]
async fn wrong_methods_are_rejected_with_405() {
    let test = test_app(1024, None);

    for (method, path) in [
        (Method::POST, "/healthz"),
        (Method::GET, "/v1/responses"),
        (Method::POST, "/v1/models"),
    ] {
        let response = send(&test.app, request(method, path, "{}")).await;
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED, "{path}");
        assert!(response.headers().contains_key("x-request-id"), "{path}");
    }

    assert_eq!(test.executor.calls(), 0);
}

#[tokio::test]
async fn json_posts_reject_wrong_content_type_and_malformed_json() {
    let test = test_app(1024, None);
    let wrong_type = Request::builder()
        .method(Method::POST)
        .uri("/v1/responses")
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from("{}"))
        .unwrap();
    let response = send(&test.app, wrong_type).await;
    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

    let response = send(
        &test.app,
        request(Method::POST, "/v1/responses", "{not-json"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(test.executor.calls(), 0);
}

#[tokio::test]
async fn oversized_json_is_rejected_before_executor_runs() {
    let test = test_app(32, None);
    let response = send(
        &test.app,
        request(Method::POST, "/v1/responses", vec![b'x'; 33]),
    )
    .await;

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(test.executor.calls(), 0);
}

#[tokio::test]
async fn request_id_is_preserved_when_valid_and_replaced_when_invalid() {
    let test = test_app(1024, None);
    let accepted = Request::builder()
        .method(Method::GET)
        .uri("/healthz")
        .header("x-request-id", "req_123.alpha-9")
        .body(Body::empty())
        .unwrap();
    let response = send(&test.app, accepted).await;
    assert_eq!(response.headers()["x-request-id"], "req_123.alpha-9");

    let rejected = Request::builder()
        .method(Method::GET)
        .uri("/healthz")
        .header("x-request-id", "contains spaces")
        .body(Body::empty())
        .unwrap();
    let response = send(&test.app, rejected).await;
    let replacement = response.headers()["x-request-id"].to_str().unwrap();
    assert_ne!(replacement, "contains spaces");
    assert!(uuid::Uuid::parse_str(replacement).is_ok());

    let boundary = "a".repeat(128);
    let accepted = Request::builder()
        .method(Method::GET)
        .uri("/healthz")
        .header("x-request-id", &boundary)
        .body(Body::empty())
        .unwrap();
    let response = send(&test.app, accepted).await;
    assert_eq!(response.headers()["x-request-id"], boundary);

    let too_long = "a".repeat(129);
    let rejected = Request::builder()
        .method(Method::GET)
        .uri("/healthz")
        .header("x-request-id", &too_long)
        .body(Body::empty())
        .unwrap();
    let response = send(&test.app, rejected).await;
    let replacement = response.headers()["x-request-id"].to_str().unwrap();
    assert_ne!(replacement, too_long);
    assert!(uuid::Uuid::parse_str(replacement).is_ok());
}

#[tokio::test]
async fn bearer_auth_is_required_compared_and_stripped_before_handlers() {
    let secret = "lan-token-that-must-not-leak";
    let test = test_app(1024, Some(secret));

    let response = send(&test.app, request(Method::POST, "/v1/responses", "{}")).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(response.headers().contains_key("x-request-id"));
    let body = json_body(response).await;
    assert_eq!(body["error"]["code"], "unauthorized");
    assert_eq!(body["error"]["type"], "gateway_error");

    let wrong = Request::builder()
        .method(Method::POST)
        .uri("/v1/responses")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, "Bearer wrong-token")
        .body(Body::from("{}"))
        .unwrap();
    let response = send(&test.app, wrong).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let valid = Request::builder()
        .method(Method::POST)
        .uri("/v1/responses")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {secret}"))
        .body(Body::from("{}"))
        .unwrap();
    let response = send(&test.app, valid).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    let rendered = String::from_utf8(body.to_vec()).unwrap();
    assert!(!rendered.contains(secret));
    assert!(!rendered.to_ascii_lowercase().contains("authorization"));
    assert_eq!(test.executor.calls(), 0);
}

#[tokio::test]
async fn errors_use_safe_typed_envelopes_with_request_id() {
    let test = test_app(1024, None);
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/responses")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-request-id", "stable-id")
        .body(Body::from("{}"))
        .unwrap();
    let response = send(&test.app, request).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = json_body(response).await;

    assert_eq!(body["error"]["type"], "gateway_error");
    assert_eq!(body["error"]["code"], "unsupported_capability");
    assert_eq!(body["error"]["request_id"], "stable-id");
    assert_eq!(
        body["error"]["message"],
        "The requested capability is not supported."
    );
}

#[test]
fn listener_security_accepts_loopback_and_restricts_private_lan() {
    let token_ref = SecretRef::new();

    for ip in [Ipv4Addr::LOCALHOST.into(), Ipv6Addr::LOCALHOST.into()] {
        assert!(ListenerSecurity::validate(SocketAddr::new(ip, 10101), None, None, false).is_ok());
    }

    for ip in [
        Ipv4Addr::new(10, 0, 0, 2).into(),
        Ipv4Addr::new(172, 16, 0, 2).into(),
        Ipv4Addr::new(192, 168, 0, 2).into(),
        "fd00::2".parse().unwrap(),
    ] {
        let address = SocketAddr::new(ip, 10101);
        assert!(ListenerSecurity::validate(address, None, None, true).is_err());
        assert!(ListenerSecurity::validate(address, Some(&token_ref), None, false).is_err());
        assert!(ListenerSecurity::validate(address, Some(&token_ref), None, true).is_ok());
    }
}

#[test]
fn listener_security_rejects_public_unspecified_multicast_and_link_local() {
    let token_ref = SecretRef::new();

    for ip in [
        Ipv4Addr::UNSPECIFIED.into(),
        Ipv4Addr::new(8, 8, 8, 8).into(),
        Ipv4Addr::new(169, 254, 1, 1).into(),
        Ipv4Addr::new(224, 0, 0, 1).into(),
        Ipv6Addr::UNSPECIFIED.into(),
        "2001:4860:4860::8888".parse().unwrap(),
        "fe80::1".parse().unwrap(),
        "ff02::1".parse().unwrap(),
    ] {
        assert!(
            ListenerSecurity::validate(SocketAddr::new(ip, 10101), Some(&token_ref), None, true,)
                .is_err(),
            "{ip}"
        );
    }
}

#[test]
fn valid_tls_paths_allow_private_lan_without_insecure_ack() {
    let directory = tempfile::tempdir().unwrap();
    let tls = valid_tls_config(&directory);
    let token_ref = SecretRef::new();

    let validated = ListenerSecurity::validate(
        "192.168.1.5:10101".parse().unwrap(),
        Some(&token_ref),
        Some(&tls),
        false,
    )
    .unwrap();

    assert!(validated.rustls_config().is_some());
}

#[test]
fn tls_paths_reject_same_missing_directory_and_unparseable_files() {
    let directory = tempfile::tempdir().unwrap();
    let same = directory.path().join("same.pem");
    std::fs::write(&same, CERTIFICATE_PEM).unwrap();
    let error = TlsConfig::new(&same, &same).validate().unwrap_err();
    assert!(!format!("{error:?}").contains(PRIVATE_KEY_PEM));

    let missing = directory.path().join("missing.pem");
    assert!(
        TlsConfig::new(&missing, directory.path().join("key.pem"))
            .validate()
            .is_err()
    );
    assert!(
        TlsConfig::new(directory.path(), directory.path().join("key.pem"))
            .validate()
            .is_err()
    );

    let cert = directory.path().join("cert.pem");
    let key = directory.path().join("key.pem");
    std::fs::write(&cert, "not a certificate").unwrap();
    std::fs::write(&key, "not a private key").unwrap();
    let error = TlsConfig::new(&cert, &key).validate().unwrap_err();
    let rendered = format!("{error:?}");
    assert!(!rendered.contains("not a private key"));
    assert!(!rendered.contains(PRIVATE_KEY_PEM));

    std::fs::write(&cert, CERTIFICATE_PEM).unwrap();
    let error = TlsConfig::new(&cert, &key).validate().unwrap_err();
    let rendered = format!("{error:?}");
    assert!(!rendered.contains("not a private key"));
    assert!(!rendered.contains(PRIVATE_KEY_PEM));

    std::fs::write(&key, "").unwrap();
    assert!(TlsConfig::new(&cert, &key).validate().is_err());
}

fn valid_tls_config(directory: &TempDir) -> TlsConfig {
    let certificate_path = directory.path().join("certificate.pem");
    let private_key_path = directory.path().join("private-key.pem");
    std::fs::write(&certificate_path, CERTIFICATE_PEM).unwrap();
    std::fs::write(&private_key_path, PRIVATE_KEY_PEM).unwrap();
    TlsConfig::new(certificate_path, private_key_path)
}
