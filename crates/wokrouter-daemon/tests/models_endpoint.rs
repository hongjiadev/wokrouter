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
use futures::{TryStreamExt, stream};
use serde_json::Value;
use tower::ServiceExt;
use wokrouter_daemon::data_plane::{
    AdapterRegistry, CanonicalStream, DataPlaneState, ExecutionContext, FrontDoorMetric,
    ImmutableSnapshot, MetricsSink, ModelCatalogSnapshot, PublicModelMetadata, RequestLimits,
    UpstreamExecutor, build_data_plane,
};
use wokrouter_protocols::canonical::{
    AdapterKind, CanonicalEvent, CanonicalRequest, GatewayError, PublicModelId, RequestId,
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

struct ExpectedExecutor {
    expected_context_request_id: RequestId,
    expected_snapshot: Arc<dyn ImmutableSnapshot>,
    expected_request: CanonicalRequest,
    response_id: String,
    calls: AtomicUsize,
}

#[async_trait]
impl UpstreamExecutor for ExpectedExecutor {
    async fn execute(
        &self,
        context: ExecutionContext,
        request: CanonicalRequest,
    ) -> Result<CanonicalStream, GatewayError> {
        assert_eq!(context.request_id, self.expected_context_request_id);
        assert!(Arc::ptr_eq(&context.snapshot, &self.expected_snapshot));
        assert_eq!(request, self.expected_request);
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Box::pin(stream::iter([
            Ok(CanonicalEvent::Created {
                response_id: self.response_id.clone(),
            }),
            Ok(CanonicalEvent::Completed),
        ])))
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

#[test]
fn model_catalog_deduplicates_identical_entries_and_rejects_conflicts() {
    let model = PublicModelMetadata::new("same-model", "wokrouter").with_capability("tools", true);
    let catalog = ModelCatalogSnapshot::new(vec![model.clone(), model]).unwrap();
    assert_eq!(catalog.models().len(), 1);

    assert_eq!(
        ModelCatalogSnapshot::new(vec![
            PublicModelMetadata::new("same-model", "owner-a"),
            PublicModelMetadata::new("same-model", "owner-b"),
        ])
        .unwrap_err()
        .code(),
        "invalid_request"
    );
    assert_eq!(
        ModelCatalogSnapshot::new(vec![
            PublicModelMetadata::new("same-model", "wokrouter").with_capability("tools", true),
            PublicModelMetadata::new("same-model", "wokrouter").with_capability("tools", false),
        ])
        .unwrap_err()
        .code(),
        "invalid_request"
    );
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

async fn assert_registry_forwarding(kind: AdapterKind, label: &str) {
    let snapshot: Arc<dyn ImmutableSnapshot> = Arc::new(CatalogSnapshot {
        catalog: ModelCatalogSnapshot::default(),
    });
    let expected_request = canonical_request();
    let expected_context_request_id = RequestId::new(format!("req_{label}"));
    let response_id = format!("response_{label}");
    let executor = Arc::new(ExpectedExecutor {
        expected_context_request_id: expected_context_request_id.clone(),
        expected_snapshot: Arc::clone(&snapshot),
        expected_request: expected_request.clone(),
        response_id: response_id.clone(),
        calls: AtomicUsize::new(0),
    });
    let mut registry = AdapterRegistry::new();
    registry.register(kind, executor.clone());

    let events = registry
        .execute(
            kind,
            ExecutionContext {
                request_id: expected_context_request_id,
                snapshot,
            },
            expected_request,
        )
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();

    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        events,
        [
            CanonicalEvent::Created { response_id },
            CanonicalEvent::Completed,
        ]
    );
}

macro_rules! registry_forwarding_test {
    ($name:ident, $kind:expr, $label:literal) => {
        #[tokio::test]
        async fn $name() {
            assert_registry_forwarding($kind, $label).await;
        }
    };
}

registry_forwarding_test!(
    registry_forwards_openai_responses,
    AdapterKind::OpenAiResponses,
    "openai_responses"
);
registry_forwarding_test!(
    registry_forwards_openai_chat,
    AdapterKind::OpenAiChat,
    "openai_chat"
);
registry_forwarding_test!(
    registry_forwards_anthropic,
    AdapterKind::Anthropic,
    "anthropic"
);
registry_forwarding_test!(registry_forwards_gemini, AdapterKind::Gemini, "gemini");
registry_forwarding_test!(
    registry_forwards_azure_openai,
    AdapterKind::AzureOpenAi,
    "azure_openai"
);
registry_forwarding_test!(registry_forwards_cursor, AdapterKind::Cursor, "cursor");

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
