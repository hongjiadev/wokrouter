use axum::{
    body::Bytes,
    extract::{FromRequest, Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::Response,
};
use secrecy::ExposeSecret;
use subtle::{Choice, ConstantTimeEq};
use uuid::Uuid;
use wokrouter_protocols::canonical::{GatewayError, RequestId};

use super::{
    registry::{ClientProtocol, ProtocolRegistry},
    response::{gateway_error_response, public_error_response},
    router::{DataPlaneState, FrontDoorMetric},
};

const REQUEST_ID_HEADER: &str = "x-request-id";
const MAX_REQUEST_ID_BYTES: usize = 128;
const MAX_BEARER_TOKEN_BYTES: usize = 1024;

pub(crate) struct ValidatedJsonBody;

impl<S> FromRequest<S> for ValidatedJsonBody
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let request_id = request
            .extensions()
            .get::<RequestId>()
            .expect("front-door middleware must assign a request ID")
            .clone();
        let protocol = *request
            .extensions()
            .get::<ClientProtocol>()
            .expect("the protocol registry must classify every JSON route");
        let body = Bytes::from_request(request, state)
            .await
            .map_err(|rejection| {
                let status = rejection.status();
                if status == StatusCode::PAYLOAD_TOO_LARGE {
                    return public_error_response(
                        status,
                        "payload_too_large",
                        "The request body exceeds the configured limit.",
                        &request_id,
                        Some(protocol),
                    );
                }
                public_error_response(
                    status,
                    "invalid_body",
                    "The request body could not be read.",
                    &request_id,
                    Some(protocol),
                )
            })?;
        serde_json::from_slice::<serde_json::Value>(&body)
            .map_err(|_| {
                gateway_error_response(GatewayError::invalid_request(), &request_id, protocol)
            })
            .map(|_| Self)
    }
}

pub(crate) async fn front_door(
    State(state): State<DataPlaneState>,
    mut request: Request,
    next: Next,
) -> Response {
    let request_id = request_id(request.headers().get(REQUEST_ID_HEADER));
    request.extensions_mut().insert(request_id.clone());
    let protocol = ProtocolRegistry::resolve(request.uri().path());
    if let Some(protocol) = protocol {
        request.extensions_mut().insert(protocol);
    }

    let method = request.method().to_string();
    let path = request.uri().path().to_owned();
    let authorization = request.headers_mut().remove(header::AUTHORIZATION);
    let authorized = is_authorized(&state, authorization.as_ref());
    drop(authorization);

    let mut response = if !authorized {
        public_error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "The request is not authorized.",
            &request_id,
            protocol,
        )
    } else if requires_json_content_type(request.method(), request.uri().path())
        && !is_json_content_type(request.headers().get(header::CONTENT_TYPE))
    {
        public_error_response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "Content-Type must be application/json.",
            &request_id,
            protocol,
        )
    } else {
        next.run(request).await
    };

    if let Ok(value) = HeaderValue::from_str(request_id.as_str()) {
        response.headers_mut().insert(REQUEST_ID_HEADER, value);
    }
    state.metrics().record(FrontDoorMetric {
        request_id,
        method,
        path,
        status: response.status().as_u16(),
        snapshot_revision: state.snapshot().revision(),
    });
    response
}

fn request_id(value: Option<&HeaderValue>) -> RequestId {
    value
        .and_then(|value| value.to_str().ok())
        .filter(|value| valid_request_id(value))
        .map(RequestId::new)
        .unwrap_or_else(|| RequestId::new(Uuid::new_v4().to_string()))
}

fn valid_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_REQUEST_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn is_authorized(state: &DataPlaneState, authorization: Option<&HeaderValue>) -> bool {
    let Some(expected) = state.lan_bearer() else {
        return true;
    };
    let Some(provided) = authorization
        .and_then(|value| value.to_str().ok())
        .and_then(bearer_credential)
    else {
        return false;
    };

    constant_time_token_eq(provided.as_bytes(), expected.expose_secret().as_bytes())
}

fn bearer_credential(value: &str) -> Option<&str> {
    let (scheme, credential) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("Bearer")
        || credential.is_empty()
        || credential.bytes().any(|byte| byte.is_ascii_whitespace())
    {
        return None;
    }
    Some(credential)
}

fn constant_time_token_eq(provided: &[u8], expected: &[u8]) -> bool {
    let mut provided_padded = [0_u8; MAX_BEARER_TOKEN_BYTES];
    let mut expected_padded = [0_u8; MAX_BEARER_TOKEN_BYTES];
    for index in 0..MAX_BEARER_TOKEN_BYTES {
        provided_padded[index] = provided.get(index).copied().unwrap_or_default();
        expected_padded[index] = expected.get(index).copied().unwrap_or_default();
    }

    let contents_match = provided_padded.ct_eq(&expected_padded);
    let lengths_match = provided.len().ct_eq(&expected.len());
    let provided_in_bounds = Choice::from((provided.len() <= MAX_BEARER_TOKEN_BYTES) as u8);
    let expected_in_bounds = Choice::from((expected.len() <= MAX_BEARER_TOKEN_BYTES) as u8);
    bool::from(contents_match & lengths_match & provided_in_bounds & expected_in_bounds)
}

fn requires_json_content_type(method: &Method, path: &str) -> bool {
    method == Method::POST
        && ProtocolRegistry::resolve(path).is_some_and(ClientProtocol::expects_json_body)
}

fn is_json_content_type(value: Option<&HeaderValue>) -> bool {
    let Some(media_type) = value
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
    else {
        return false;
    };

    media_type.eq_ignore_ascii_case("application/json")
        || media_type
            .to_ascii_lowercase()
            .strip_prefix("application/")
            .is_some_and(|subtype| subtype.ends_with("+json"))
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use axum::{
        Extension, Router,
        body::Body,
        http::{HeaderMap, Method, Request, StatusCode, header},
        middleware,
        routing::post,
    };
    use futures::stream;
    use secrecy::SecretString;
    use tower::ServiceExt;
    use wokrouter_protocols::canonical::{CanonicalRequest, GatewayError};

    use super::{MAX_BEARER_TOKEN_BYTES, ValidatedJsonBody, constant_time_token_eq, front_door};
    use crate::data_plane::{
        CanonicalStream, DataPlaneState, ExecutionContext, FrontDoorMetric, ImmutableSnapshot,
        MetricsSink, RequestLimits, UpstreamExecutor,
    };

    #[test]
    fn constant_time_token_comparison_accepts_an_exact_match() {
        assert!(constant_time_token_eq(b"correct-token", b"correct-token"));
    }

    #[test]
    fn constant_time_token_comparison_rejects_an_equal_length_mismatch() {
        assert!(!constant_time_token_eq(b"correct-token", b"wrong---token"));
    }

    #[test]
    fn constant_time_token_comparison_rejects_a_short_value() {
        assert!(!constant_time_token_eq(b"short", b"correct-token"));
    }

    #[test]
    fn constant_time_token_comparison_rejects_a_long_value() {
        assert!(!constant_time_token_eq(
            b"correct-token-extra",
            b"correct-token"
        ));
    }

    #[test]
    fn constant_time_token_comparison_rejects_values_over_the_fixed_limit() {
        let over_limit = vec![b'x'; MAX_BEARER_TOKEN_BYTES + 1];
        assert!(!constant_time_token_eq(&over_limit, &over_limit));
    }

    #[derive(Default)]
    struct TerminalProbe {
        calls: AtomicUsize,
        saw_authorization: AtomicBool,
    }

    struct TestSnapshot;

    impl ImmutableSnapshot for TestSnapshot {
        fn revision(&self) -> u64 {
            1
        }
    }

    struct TestMetrics;

    impl MetricsSink for TestMetrics {
        fn record(&self, _metric: FrontDoorMetric) {}
    }

    struct NeverExecutor;

    #[async_trait]
    impl UpstreamExecutor for NeverExecutor {
        async fn execute(
            &self,
            _context: ExecutionContext,
            _request: CanonicalRequest,
        ) -> Result<CanonicalStream, GatewayError> {
            Ok(Box::pin(stream::empty()))
        }
    }

    fn test_pipeline() -> (Router, Arc<TerminalProbe>) {
        let probe = Arc::new(TerminalProbe::default());
        let state = DataPlaneState::new(
            Arc::new(NeverExecutor),
            RequestLimits { json_body_bytes: 8 },
            Arc::new(TestSnapshot),
            Arc::new(TestMetrics),
        )
        .with_lan_bearer(SecretString::from("pipeline-secret".to_owned()));
        let router = Router::new()
            .route("/v1/responses", post(terminal))
            .with_state(state.clone())
            .layer(axum::extract::DefaultBodyLimit::max(8))
            .layer(Extension(Arc::clone(&probe)))
            .layer(middleware::from_fn_with_state(state, front_door));
        (router, probe)
    }

    async fn terminal(
        headers: HeaderMap,
        Extension(probe): Extension<Arc<TerminalProbe>>,
        _body: ValidatedJsonBody,
    ) -> StatusCode {
        probe.calls.fetch_add(1, Ordering::SeqCst);
        probe.saw_authorization.store(
            headers.contains_key(header::AUTHORIZATION),
            Ordering::SeqCst,
        );
        StatusCode::NO_CONTENT
    }

    fn pipeline_request(
        body: &'static str,
        content_type: &'static str,
        authorization: Option<&'static str>,
    ) -> Request<Body> {
        let mut request = Request::builder()
            .method(Method::POST)
            .uri("/v1/responses")
            .header(header::CONTENT_TYPE, content_type);
        if let Some(authorization) = authorization {
            request = request.header(header::AUTHORIZATION, authorization);
        }
        request.body(Body::from(body)).unwrap()
    }

    #[tokio::test]
    async fn valid_pipeline_request_reaches_terminal_without_authorization() {
        let (router, probe) = test_pipeline();
        let response = router
            .oneshot(pipeline_request(
                "{}",
                "application/json",
                Some("Bearer pipeline-secret"),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
        assert!(!probe.saw_authorization.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn unauthorized_pipeline_request_does_not_reach_terminal() {
        let (router, probe) = test_pipeline();
        let response = router
            .oneshot(pipeline_request("{}", "application/json", None))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn unsupported_content_type_does_not_reach_terminal() {
        let (router, probe) = test_pipeline();
        let response = router
            .oneshot(pipeline_request(
                "{}",
                "text/plain",
                Some("Bearer pipeline-secret"),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn oversized_body_does_not_reach_terminal() {
        let (router, probe) = test_pipeline();
        let response = router
            .oneshot(pipeline_request(
                "123456789",
                "application/json",
                Some("Bearer pipeline-secret"),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn invalid_json_does_not_reach_terminal() {
        let (router, probe) = test_pipeline();
        let response = router
            .oneshot(pipeline_request(
                "{",
                "application/json",
                Some("Bearer pipeline-secret"),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
    }
}
