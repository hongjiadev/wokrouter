use axum::{
    Json,
    extract::Extension,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use wokrouter_protocols::canonical::{GatewayError, RequestId};

use super::{extract::ValidatedJsonBody, registry::ClientProtocol};

#[derive(Serialize)]
struct OpenAiErrorEnvelope<'a> {
    error: OpenAiPublicError<'a>,
}

#[derive(Serialize)]
struct OpenAiPublicError<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    code: &'a str,
    message: &'a str,
    request_id: &'a str,
}

#[derive(Serialize)]
struct AnthropicErrorEnvelope<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    error: AnthropicPublicError<'a>,
    request_id: &'a str,
}

#[derive(Serialize)]
struct AnthropicPublicError<'a> {
    #[serde(rename = "type")]
    code: &'a str,
    message: &'a str,
}

pub(crate) async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

pub(crate) async fn unsupported(
    Extension(request_id): Extension<RequestId>,
    Extension(protocol): Extension<ClientProtocol>,
) -> Response {
    gateway_error_response(
        GatewayError::unsupported_capability(),
        &request_id,
        protocol,
    )
}

pub(crate) async fn unsupported_json(
    Extension(request_id): Extension<RequestId>,
    Extension(protocol): Extension<ClientProtocol>,
    _body: ValidatedJsonBody,
) -> Response {
    gateway_error_response(
        GatewayError::unsupported_capability(),
        &request_id,
        protocol,
    )
}

pub(crate) fn gateway_error_response(
    error: GatewayError,
    request_id: &RequestId,
    protocol: ClientProtocol,
) -> Response {
    let status =
        StatusCode::from_u16(error.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    public_error_response(
        status,
        error.code(),
        error.public_message(),
        request_id,
        Some(protocol),
    )
}

pub(crate) fn public_error_response(
    status: StatusCode,
    code: &str,
    message: &str,
    request_id: &RequestId,
    protocol: Option<ClientProtocol>,
) -> Response {
    if protocol.is_some_and(ClientProtocol::is_anthropic) {
        return (
            status,
            Json(AnthropicErrorEnvelope {
                kind: "error",
                error: AnthropicPublicError { code, message },
                request_id: request_id.as_str(),
            }),
        )
            .into_response();
    }

    (
        status,
        Json(OpenAiErrorEnvelope {
            error: OpenAiPublicError {
                kind: "gateway_error",
                code,
                message,
                request_id: request_id.as_str(),
            },
        }),
    )
        .into_response()
}
