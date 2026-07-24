use axum::{
    extract::{Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::Response,
};
use secrecy::ExposeSecret;
use subtle::ConstantTimeEq;
use uuid::Uuid;
use wokrouter_protocols::canonical::RequestId;

use super::{
    response::public_error_response,
    router::{DataPlaneState, FrontDoorMetric},
};

const REQUEST_ID_HEADER: &str = "x-request-id";
const MAX_REQUEST_ID_BYTES: usize = 128;

pub(crate) async fn front_door(
    State(state): State<DataPlaneState>,
    mut request: Request,
    next: Next,
) -> Response {
    let request_id = request_id(request.headers().get(REQUEST_ID_HEADER));
    request.extensions_mut().insert(request_id.clone());

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
        )
    } else if requires_json_content_type(request.method(), request.uri().path())
        && !is_json_content_type(request.headers().get(header::CONTENT_TYPE))
    {
        public_error_response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
            "Content-Type must be application/json.",
            &request_id,
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
        .and_then(|value| value.strip_prefix("Bearer "))
    else {
        return false;
    };

    bool::from(
        provided
            .as_bytes()
            .ct_eq(expected.expose_secret().as_bytes()),
    )
}

fn requires_json_content_type(method: &Method, path: &str) -> bool {
    method == Method::POST
        && matches!(
            path,
            "/v1/responses"
                | "/v1/chat/completions"
                | "/v1/messages"
                | "/v1/messages/count_tokens"
                | "/v1/images/generations"
                | "/v1/images/edits"
        )
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
