use std::{pin::Pin, sync::Arc};

use async_trait::async_trait;
use axum::{
    Router, middleware,
    routing::{get, post},
};
use futures::Stream;
use secrecy::SecretString;
use wokrouter_protocols::canonical::{CanonicalEvent, CanonicalRequest, GatewayError, RequestId};

use super::{ModelCatalogSnapshot, extract::front_door, models, response};

pub const DEFAULT_JSON_BODY_BYTES: usize = 16 * 1024 * 1024;

pub type CanonicalStream = Pin<Box<dyn Stream<Item = Result<CanonicalEvent, GatewayError>> + Send>>;

pub trait ImmutableSnapshot: Send + Sync {
    fn revision(&self) -> u64;

    fn model_catalog(&self) -> ModelCatalogSnapshot {
        ModelCatalogSnapshot::default()
    }
}

#[derive(Clone)]
pub struct ExecutionContext {
    pub request_id: RequestId,
    pub snapshot: Arc<dyn ImmutableSnapshot>,
}

#[async_trait]
pub trait UpstreamExecutor: Send + Sync {
    async fn execute(
        &self,
        context: ExecutionContext,
        request: CanonicalRequest,
    ) -> Result<CanonicalStream, GatewayError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestLimits {
    pub json_body_bytes: usize,
}

impl Default for RequestLimits {
    fn default() -> Self {
        Self {
            json_body_bytes: DEFAULT_JSON_BODY_BYTES,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrontDoorMetric {
    pub request_id: RequestId,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub snapshot_revision: u64,
}

pub trait MetricsSink: Send + Sync {
    fn record(&self, metric: FrontDoorMetric);
}

#[derive(Clone)]
pub struct DataPlaneState {
    executor: Arc<dyn UpstreamExecutor>,
    request_limits: RequestLimits,
    snapshot: Arc<dyn ImmutableSnapshot>,
    metrics: Arc<dyn MetricsSink>,
    lan_bearer: Option<Arc<SecretString>>,
}

impl DataPlaneState {
    pub fn new(
        executor: Arc<dyn UpstreamExecutor>,
        request_limits: RequestLimits,
        snapshot: Arc<dyn ImmutableSnapshot>,
        metrics: Arc<dyn MetricsSink>,
    ) -> Self {
        Self {
            executor,
            request_limits,
            snapshot,
            metrics,
            lan_bearer: None,
        }
    }

    pub fn with_lan_bearer(mut self, bearer: SecretString) -> Self {
        self.lan_bearer = Some(Arc::new(bearer));
        self
    }

    pub fn executor(&self) -> &Arc<dyn UpstreamExecutor> {
        &self.executor
    }

    pub fn request_limits(&self) -> RequestLimits {
        self.request_limits
    }

    pub fn snapshot(&self) -> &Arc<dyn ImmutableSnapshot> {
        &self.snapshot
    }

    pub fn metrics(&self) -> &Arc<dyn MetricsSink> {
        &self.metrics
    }

    pub fn execution_context(&self, request_id: RequestId) -> ExecutionContext {
        ExecutionContext {
            request_id,
            snapshot: Arc::clone(&self.snapshot),
        }
    }

    pub(crate) fn lan_bearer(&self) -> Option<&SecretString> {
        self.lan_bearer.as_deref()
    }
}

pub fn build_data_plane(state: DataPlaneState) -> Router {
    let body_limit = state.request_limits.json_body_bytes;
    Router::new()
        .route("/healthz", get(response::health))
        .route("/v1/responses", post(response::unsupported_json))
        .route("/v1/chat/completions", post(response::unsupported_json))
        .route("/v1/messages", post(response::unsupported_json))
        .route(
            "/v1/messages/count_tokens",
            post(response::unsupported_json),
        )
        .route("/v1/models", get(models::models))
        .route("/v1/images/generations", post(response::unsupported_json))
        .route("/v1/images/edits", post(response::unsupported_json))
        .with_state(state.clone())
        .layer(axum::extract::DefaultBodyLimit::max(body_limit))
        .layer(middleware::from_fn_with_state(state, front_door))
}
