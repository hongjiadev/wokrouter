use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use futures::stream;
use serde_json::Value;
use tower::ServiceExt;
use wokrouter_daemon::data_plane::{
    AdapterRegistry, CanonicalStream, DataPlaneState, ExecutionContext, FrontDoorMetric,
    ImmutableSnapshot, MetricsSink, ModelCatalogSnapshot, PublicModelMetadata, RequestLimits,
    UpstreamExecutor, build_data_plane,
};
use wokrouter_protocols::canonical::{
    AdapterKind, CanonicalRequest, GatewayError, PublicModelId, RequestId,
};

#[derive(Clone)]
struct CatalogSnapshot {
    catalog: ModelCatalogSnapshot,
}

impl ImmutableSnapshot for CatalogSnapshot {
    fn revision(&self) -> u64 {
        11
    }

    fn model_catalog(&self) -> ModelCatalogSnapshot {
        self.catalog.clone()
    }
}

#[derive(Default)]
struct NoopMetrics;

impl MetricsSink for NoopMetrics {
    fn record(&self, _metric: FrontDoorMetric) {}
}

#[derive(Clone, Default)]
struct RecordingExecutor {
    calls: Arc<AtomicUsize>,
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

fn canonical_request() -> CanonicalRequest {
    CanonicalRequest {
        request_id: RequestId::new("req_registry"),
        model: PublicModelId::new("public-model"),
        thread_key: None,
        input: Vec::new(),
        tools: Vec::new(),
        stream: false,
        reasoning: None,
        extensions: BTreeMap::new(),
    }
}

#[tokio::test]
async fn models_are_sorted_and_expose_only_public_metadata() {
    let catalog = ModelCatalogSnapshot::new(vec![
        PublicModelMetadata::new("z-model", "wokrouter").with_capability("tools", true),
        PublicModelMetadata::new("a-model", "wokrouter").with_capability("reasoning", true),
    ])
    .unwrap();
    let state = DataPlaneState::new(
        Arc::new(RecordingExecutor::default()),
        RequestLimits::default(),
        Arc::new(CatalogSnapshot { catalog }),
        Arc::new(NoopMetrics),
    );
    let response = build_data_plane(state)
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(body["object"], "list");
    assert_eq!(
        body["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|model| model["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["a-model", "z-model"]
    );
    assert_eq!(body["data"][0]["object"], "model");
    assert_eq!(body["data"][0]["owned_by"], "wokrouter");
    assert_eq!(body["data"][0]["capabilities"]["reasoning"], true);

    let serialized = String::from_utf8(bytes.to_vec()).unwrap();
    for private in [
        "fixture-secret",
        "internal.example.test",
        "deployment-a",
        "api-version",
        "account_id",
    ] {
        assert!(!serialized.contains(private));
    }
}

#[tokio::test]
async fn adapter_registry_dispatches_every_defined_kind() {
    let executor = RecordingExecutor::default();
    let mut registry = AdapterRegistry::new();
    for kind in [
        AdapterKind::OpenAiResponses,
        AdapterKind::OpenAiChat,
        AdapterKind::Anthropic,
        AdapterKind::Gemini,
        AdapterKind::AzureOpenAi,
        AdapterKind::Cursor,
    ] {
        registry.register(kind, Arc::new(executor.clone()));
    }
    let snapshot: Arc<dyn ImmutableSnapshot> = Arc::new(CatalogSnapshot {
        catalog: ModelCatalogSnapshot::default(),
    });
    for kind in [
        AdapterKind::OpenAiResponses,
        AdapterKind::OpenAiChat,
        AdapterKind::Anthropic,
        AdapterKind::Gemini,
        AdapterKind::AzureOpenAi,
        AdapterKind::Cursor,
    ] {
        let _stream = registry
            .execute(
                kind,
                ExecutionContext {
                    request_id: RequestId::new("req_registry"),
                    snapshot: Arc::clone(&snapshot),
                },
                canonical_request(),
            )
            .await
            .unwrap();
    }
    assert_eq!(executor.calls.load(Ordering::SeqCst), 6);
}

#[tokio::test]
async fn missing_executor_returns_stable_no_executor() {
    let registry = AdapterRegistry::new();
    let snapshot: Arc<dyn ImmutableSnapshot> = Arc::new(CatalogSnapshot {
        catalog: ModelCatalogSnapshot::default(),
    });
    let error = registry
        .execute(
            AdapterKind::Cursor,
            ExecutionContext {
                request_id: RequestId::new("req_no_executor"),
                snapshot,
            },
            canonical_request(),
        )
        .await
        .err()
        .unwrap();
    assert_eq!(error.code(), "no_executor");
}
