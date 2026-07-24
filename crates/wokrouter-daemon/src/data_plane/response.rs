use axum::{
    Json,
    body::Bytes,
    extract::{Extension, rejection::BytesRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use wokrouter_protocols::canonical::{GatewayError, RequestId};

#[derive(Serialize)]
struct ErrorEnvelope<'a> {
    error: PublicError<'a>,
}

#[derive(Serialize)]
struct PublicError<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    code: &'a str,
    message: &'a str,
    request_id: &'a str,
}

pub(crate) async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

pub(crate) async fn unsupported(Extension(request_id): Extension<RequestId>) -> Response {
    gateway_error_response(GatewayError::unsupported_capability(), &request_id)
}

pub(crate) async fn unsupported_json(
    Extension(request_id): Extension<RequestId>,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let body = match body {
        Ok(body) => body,
        Err(rejection) => {
            let status = rejection.status();
            if status == StatusCode::PAYLOAD_TOO_LARGE {
                return public_error_response(
                    status,
                    "payload_too_large",
                    "The request body exceeds the configured limit.",
                    &request_id,
                );
            }
            return public_error_response(
                status,
                "invalid_body",
                "The request body could not be read.",
                &request_id,
            );
        }
    };

    if serde_json::from_slice::<serde_json::Value>(&body).is_err() {
        return gateway_error_response(GatewayError::invalid_request(), &request_id);
    }

    gateway_error_response(GatewayError::unsupported_capability(), &request_id)
}

pub(crate) fn gateway_error_response(error: GatewayError, request_id: &RequestId) -> Response {
    let status =
        StatusCode::from_u16(error.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    public_error_response(status, error.code(), error.public_message(), request_id)
}

pub(crate) fn public_error_response(
    status: StatusCode,
    code: &str,
    message: &str,
    request_id: &RequestId,
) -> Response {
    (
        status,
        Json(ErrorEnvelope {
            error: PublicError {
                kind: "gateway_error",
                code,
                message,
                request_id: request_id.as_str(),
            },
        }),
    )
        .into_response()
}
