use std::collections::BTreeMap;

use serde_json::{Value, json};
use wokrouter_protocols::canonical::{
    AdapterKind, CanonicalEvent, CanonicalRequest, GatewayError, ImageDetail, InputItem,
    PublicModelId, ReasoningOptions, RequestId, RetryClass, ThreadKey, ToolDefinition, Usage,
};

fn fixture_request() -> CanonicalRequest {
    CanonicalRequest {
        request_id: RequestId::new("req_test"),
        model: PublicModelId::new("openai/gpt-test"),
        thread_key: Some(ThreadKey::new("thread_test")),
        input: vec![
            InputItem::Text {
                text: "hello".to_owned(),
            },
            InputItem::ImageUrl {
                url: "https://example.com/image.png".parse().unwrap(),
                detail: Some(ImageDetail::High),
            },
            InputItem::ToolResult {
                call_id: "call_1".to_owned(),
                output: json!({"ok": true}),
            },
        ],
        tools: vec![
            ToolDefinition {
                name: "read_file".to_owned(),
                description: Some("Read a file".to_owned()),
                input_schema: json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                }),
                extensions: BTreeMap::new(),
            }
            .with_extension("strict", json!(true)),
        ],
        stream: true,
        reasoning: Some(ReasoningOptions {
            effort: Some("high".to_owned()),
            extensions: BTreeMap::from([("summary".to_owned(), json!("auto"))]),
        }),
        extensions: BTreeMap::new(),
    }
}

#[test]
fn canonical_request_round_trips_without_losing_extensions() {
    let request = fixture_request().with_extension("vendor_field", serde_json::json!({"x": 1}));

    let encoded = serde_json::to_vec(&request).unwrap();
    let decoded: CanonicalRequest = serde_json::from_slice(&encoded).unwrap();

    assert_eq!(decoded, request);
}

#[test]
fn canonical_request_serializes_only_the_stable_roadmap_fields() {
    let encoded = serde_json::to_value(fixture_request()).unwrap();
    let keys = encoded
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();

    assert_eq!(
        keys,
        [
            "extensions",
            "input",
            "model",
            "reasoning",
            "request_id",
            "stream",
            "thread_key",
            "tools",
        ]
    );
}

#[test]
fn canonical_newtypes_are_transparent_and_minimally_accessible() {
    let request_id = RequestId::new("req_123");
    let model = PublicModelId::new("provider/model");
    let thread = ThreadKey::new("thread:123");

    assert_eq!(request_id.as_str(), "req_123");
    assert_eq!(model.as_str(), "provider/model");
    assert_eq!(thread.as_str(), "thread:123");
    assert_eq!(request_id.to_string(), "req_123");
    assert_eq!(model.to_string(), "provider/model");
    assert_eq!(thread.to_string(), "thread:123");
    assert_eq!(serde_json::to_value(&request_id).unwrap(), json!("req_123"));
    assert_eq!(
        serde_json::from_value::<RequestId>(json!("req_123")).unwrap(),
        request_id
    );
}

#[test]
fn adapter_kind_has_one_stable_wire_name_for_every_adapter() {
    let cases = [
        (AdapterKind::OpenAiResponses, "open_ai_responses"),
        (AdapterKind::OpenAiChat, "open_ai_chat"),
        (AdapterKind::Anthropic, "anthropic"),
        (AdapterKind::Gemini, "gemini"),
        (AdapterKind::AzureOpenAi, "azure_open_ai"),
        (AdapterKind::Cursor, "cursor"),
    ];

    for (adapter, wire_name) in cases {
        let encoded = serde_json::to_value(adapter).unwrap();
        assert_eq!(encoded, json!(wire_name));
        assert_eq!(
            serde_json::from_value::<AdapterKind>(encoded).unwrap(),
            adapter
        );
    }
}

#[test]
fn input_items_have_stable_tags_and_round_trip() {
    let cases = [
        (
            InputItem::Text {
                text: "hello".to_owned(),
            },
            "text",
        ),
        (
            InputItem::ImageUrl {
                url: "https://example.com/image.png".parse().unwrap(),
                detail: Some(ImageDetail::Auto),
            },
            "image_url",
        ),
        (
            InputItem::ToolResult {
                call_id: "call_1".to_owned(),
                output: json!({"content": "done"}),
            },
            "tool_result",
        ),
    ];

    for (item, tag) in cases {
        let encoded = serde_json::to_value(&item).unwrap();
        assert_eq!(encoded["type"], tag);
        assert_eq!(serde_json::from_value::<InputItem>(encoded).unwrap(), item);
    }
}

#[test]
fn image_detail_has_stable_wire_names() {
    for (detail, wire_name) in [
        (ImageDetail::Auto, "auto"),
        (ImageDetail::Low, "low"),
        (ImageDetail::High, "high"),
    ] {
        let encoded = serde_json::to_value(detail).unwrap();
        assert_eq!(encoded, json!(wire_name));
        assert_eq!(
            serde_json::from_value::<ImageDetail>(encoded).unwrap(),
            detail
        );
    }
}

#[test]
fn canonical_events_have_stable_tags_and_round_trip() {
    let usage = Usage {
        input_tokens: 10,
        output_tokens: 4,
        cached_input_tokens: Some(3),
        reasoning_tokens: Some(2),
        extensions: BTreeMap::from([("vendor_usage".to_owned(), json!({"x": 1}))]),
    };
    let cases = [
        (
            CanonicalEvent::Created {
                response_id: "response_1".to_owned(),
            },
            "created",
        ),
        (
            CanonicalEvent::OutputTextDelta {
                item_id: "item_1".to_owned(),
                delta: "hello".to_owned(),
            },
            "output_text_delta",
        ),
        (
            CanonicalEvent::ReasoningDelta {
                item_id: "item_2".to_owned(),
                delta: "thinking".to_owned(),
            },
            "reasoning_delta",
        ),
        (
            CanonicalEvent::ToolCallDelta {
                item_id: "item_3".to_owned(),
                call_id: "call_1".to_owned(),
                delta: "{\"path\":".to_owned(),
            },
            "tool_call_delta",
        ),
        (CanonicalEvent::Usage(usage), "usage"),
        (CanonicalEvent::Completed, "completed"),
        (
            CanonicalEvent::Failed(GatewayError::upstream_5xx(503)),
            "failed",
        ),
    ];

    for (event, tag) in cases {
        let encoded = serde_json::to_value(&event).unwrap();
        assert_eq!(encoded["type"], tag);
        assert_eq!(
            serde_json::from_value::<CanonicalEvent>(encoded).unwrap(),
            event
        );
    }
}

#[test]
fn gateway_error_categories_have_stable_public_contracts() {
    let cases = [
        (
            GatewayError::invalid_request(),
            "invalid_request",
            400,
            RetryClass::Never,
            "The request is invalid.",
        ),
        (
            GatewayError::unknown_model(PublicModelId::new("missing/model")),
            "model_not_found",
            404,
            RetryClass::Never,
            "The requested model is not available.",
        ),
        (
            GatewayError::unsupported_capability("computer_use"),
            "unsupported_capability",
            422,
            RetryClass::Never,
            "The requested capability is not supported.",
        ),
        (
            GatewayError::upstream_auth("Authorization: Bearer secret-token"),
            "upstream_auth",
            502,
            RetryClass::RefreshCredentials,
            "The upstream account needs to be authenticated again.",
        ),
        (
            GatewayError::rate_limited(Some(12)),
            "rate_limited",
            429,
            RetryClass::AfterDelay,
            "The request was rate limited.",
        ),
        (
            GatewayError::upstream_5xx(503),
            "upstream_error",
            502,
            RetryClass::BeforeFirstEvent,
            "The upstream service failed.",
        ),
        (
            GatewayError::transport("token=network-secret"),
            "upstream_unavailable",
            502,
            RetryClass::BeforeFirstEvent,
            "The upstream service is unavailable.",
        ),
        (
            GatewayError::internal("Bearer internal-secret"),
            "internal_error",
            500,
            RetryClass::Never,
            "An internal gateway error occurred.",
        ),
    ];

    for (error, code, status, retry_class, message) in cases {
        assert_eq!(error.code(), code);
        assert_eq!(error.http_status(), status);
        assert_eq!(error.retry_class(), retry_class);
        assert_eq!(error.public_message(), message);

        let encoded = serde_json::to_value(&error).unwrap();
        assert_eq!(encoded["type"], code);
        let decoded: GatewayError = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, error);
    }
}

#[test]
fn retry_classes_have_stable_wire_names() {
    for (retry_class, wire_name) in [
        (RetryClass::Never, "never"),
        (RetryClass::RefreshCredentials, "refresh_credentials"),
        (RetryClass::AfterDelay, "after_delay"),
        (RetryClass::BeforeFirstEvent, "before_first_event"),
    ] {
        let encoded = serde_json::to_value(retry_class).unwrap();
        assert_eq!(encoded, json!(wire_name));
        assert_eq!(
            serde_json::from_value::<RetryClass>(encoded).unwrap(),
            retry_class
        );
    }
}

#[test]
fn gateway_error_debug_display_and_serde_never_reveal_sensitive_details() {
    let secrets = [
        "Authorization: Basic dXNlcjpwYXNz",
        "Bearer secret-token",
        "access_token=provider-secret",
        "token=internal-secret",
    ];
    let errors = [
        GatewayError::upstream_auth(secrets[0]),
        GatewayError::upstream_response(502, secrets[1]),
        GatewayError::transport(secrets[2]),
        GatewayError::internal(secrets[3]),
    ];

    for error in errors {
        let outputs = [
            format!("{error:?}"),
            error.to_string(),
            serde_json::to_string(&error).unwrap(),
        ];

        for output in outputs {
            let normalized = output.to_ascii_lowercase();
            for secret in secrets {
                assert!(!normalized.contains(&secret.to_ascii_lowercase()));
            }
            assert!(!normalized.contains("authorization"));
            assert!(!normalized.contains("bearer"));
            assert!(!normalized.contains("access_token"));
            assert!(!normalized.contains("provider-secret"));
            assert!(!normalized.contains("internal-secret"));
        }
    }
}

#[test]
fn tool_reasoning_and_usage_extension_maps_round_trip() {
    let request = fixture_request();
    let encoded = serde_json::to_value(&request).unwrap();

    assert_eq!(encoded["tools"][0]["strict"], Value::Bool(true));
    assert_eq!(encoded["reasoning"]["summary"], json!("auto"));

    let decoded: CanonicalRequest = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, request);
}
