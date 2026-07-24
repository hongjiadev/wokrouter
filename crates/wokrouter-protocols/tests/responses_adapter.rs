use std::{collections::BTreeMap, fs, path::PathBuf};

use serde_json::{Value, json};
use wokrouter_protocols::{
    ResponsesCodec, UNASSIGNED_REQUEST_ID,
    canonical::{CanonicalEvent, GatewayError, ImageDetail, InputItem, RequestId, Usage},
    stream::SseDecoder,
};

fn fixture_bytes(path: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/protocols/responses")
        .join(path);
    fs::read(path).unwrap()
}

fn fixture_json(path: &str) -> Value {
    serde_json::from_slice(&fixture_bytes(path)).unwrap()
}

fn canonical_events() -> Vec<CanonicalEvent> {
    vec![
        CanonicalEvent::Created {
            response_id: "resp_fixture".to_owned(),
        },
        CanonicalEvent::OutputTextDelta {
            item_id: "msg_1".to_owned(),
            delta: "你好".to_owned(),
        },
        CanonicalEvent::ReasoningDelta {
            item_id: "reasoning_1".to_owned(),
            delta: "先检查".to_owned(),
        },
        CanonicalEvent::ToolCallDelta {
            item_id: "tool_1".to_owned(),
            call_id: "call_1".to_owned(),
            delta: "{\"path\":\"配置.toml\"}".to_owned(),
        },
        CanonicalEvent::Usage(Usage {
            input_tokens: 10,
            output_tokens: 5,
            cached_input_tokens: Some(3),
            reasoning_tokens: Some(2),
            extensions: BTreeMap::from([("vendor_usage".to_owned(), json!("kept"))]),
        }),
        CanonicalEvent::Completed,
    ]
}

#[test]
fn decodes_tool_reasoning_multimodal_and_result_fixture() {
    let request = fixture_bytes("request/tool_reasoning.json");
    let canonical = ResponsesCodec::decode_request(&request).unwrap();

    assert_eq!(canonical.request_id.as_str(), UNASSIGNED_REQUEST_ID);
    assert_eq!(canonical.model.as_str(), "gpt-test");
    assert_eq!(
        canonical.thread_key.as_ref().unwrap().as_str(),
        "resp_previous"
    );
    assert!(canonical.stream);
    assert_eq!(
        canonical.input,
        [
            InputItem::Text {
                text: "读取 配置.toml".to_owned(),
            },
            InputItem::ImageUrl {
                url: "https://example.test/image.png".parse().unwrap(),
                detail: Some(ImageDetail::High),
            },
            InputItem::ToolResult {
                call_id: "call_1".to_owned(),
                output: json!({"content": "done"}),
            },
        ]
    );
    assert_eq!(canonical.tools[0].name, "read_file");
    assert_eq!(
        canonical.tools[0].input_schema,
        json!({
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"]
        })
    );
    assert_eq!(
        canonical.tools[0].extensions,
        BTreeMap::from([("strict".to_owned(), json!(true))])
    );
    let reasoning = canonical.reasoning.unwrap();
    assert_eq!(reasoning.effort.as_deref(), Some("high"));
    assert_eq!(
        reasoning.extensions,
        BTreeMap::from([("summary".to_owned(), json!("auto"))])
    );
    assert_eq!(
        canonical.extensions["metadata"],
        json!({"tenant": "fixture"})
    );
    assert_eq!(
        canonical.extensions["responses.input_extensions"],
        json!([
            {
                "index": 0,
                "role": "user",
                "message": {"vendor_message": true},
                "content": [
                    {"index": 0, "extra": {"vendor_text": "kept"}},
                    {"index": 1, "extra": {"vendor_image": 7}}
                ]
            },
            {
                "index": 1,
                "extra": {"vendor_result": "kept"}
            }
        ])
    );
}

#[test]
fn decodes_string_input_and_allows_front_door_request_id_injection() {
    let request = fixture_bytes("request/string_input.json");
    let unassigned = ResponsesCodec::decode_request(&request).unwrap();
    let assigned =
        ResponsesCodec::decode_request_with_id(&request, RequestId::new("request-front-door"))
            .unwrap();

    assert_eq!(unassigned.request_id.as_str(), UNASSIGNED_REQUEST_ID);
    assert_eq!(assigned.request_id.as_str(), "request-front-door");
    assert_eq!(
        assigned.input,
        [InputItem::Text {
            text: "你好，world".to_owned(),
        }]
    );
}

#[test]
fn accepts_only_explicitly_allowed_image_url_schemes() {
    for url in [
        "http://example.test/image.png",
        "https://example.test/image.png",
        "data:image/png;base64,iVBORw0KGgo=",
    ] {
        let request = json!({
            "model": "gpt-test",
            "input": [{
                "role": "user",
                "content": [{"type": "input_image", "image_url": url}]
            }]
        });
        let encoded = serde_json::to_vec(&request).unwrap();
        assert!(
            ResponsesCodec::decode_request(&encoded).is_ok(),
            "{url} should be accepted"
        );
    }

    for url in [
        "file:///etc/passwd",
        "javascript:alert(1)",
        "ftp://example.test/image.png",
        "data:text/html,<script>alert(1)</script>",
    ] {
        let request = json!({
            "model": "gpt-test",
            "input": [{
                "role": "user",
                "content": [{"type": "input_image", "image_url": url}]
            }]
        });
        let encoded = serde_json::to_vec(&request).unwrap();
        assert_eq!(
            ResponsesCodec::decode_request(&encoded).unwrap_err(),
            GatewayError::invalid_request(),
            "{url} should be rejected"
        );
    }
}

#[test]
fn reserved_nested_extension_key_does_not_overwrite_top_level_extension() {
    let request = json!({
        "model": "gpt-test",
        "input": [{
            "type": "input_text",
            "text": "hello",
            "vendor_part": true
        }],
        "responses.input_extensions": "client-value"
    });
    let encoded = serde_json::to_vec(&request).unwrap();
    let canonical = ResponsesCodec::decode_request(&encoded).unwrap();

    assert_eq!(
        canonical.extensions["responses.input_extensions"],
        "client-value"
    );
    assert_eq!(
        canonical.extensions["responses.input_extensions.nested"],
        json!([{"index": 0, "extra": {"vendor_part": true}}])
    );
}

#[test]
fn rejects_missing_or_mistyped_required_fields_and_tools() {
    let cases = [
        json!({"input": "missing model"}),
        json!({"model": 7, "input": "bad model"}),
        json!({"model": "gpt-test"}),
        json!({"model": "gpt-test", "input": 7}),
        json!({
            "model": "gpt-test",
            "input": "x",
            "tools": [{"type": "function", "parameters": {"type": "object"}}]
        }),
        json!({
            "model": "gpt-test",
            "input": "x",
            "tools": [{"type": "function", "name": "", "parameters": {"type": "object"}}]
        }),
        json!({
            "model": "gpt-test",
            "input": "x",
            "tools": [{"type": "function", "name": "read_file"}]
        }),
        json!({
            "model": "gpt-test",
            "input": "x",
            "tools": [{"type": "function", "name": "read_file", "parameters": "not-schema"}]
        }),
        json!({
            "model": "gpt-test",
            "input": [{
                "type": "function_call_output",
                "call_id": "",
                "output": "x"
            }]
        }),
        json!({
            "model": "gpt-test",
            "input": [{
                "role": "user",
                "content": [{"type": "input_image", "image_url": "not a URL"}]
            }]
        }),
    ];

    for request in cases {
        let encoded = serde_json::to_vec(&request).unwrap();
        assert_eq!(
            ResponsesCodec::decode_request(&encoded).unwrap_err(),
            GatewayError::invalid_request(),
            "unexpected result for {request}"
        );
    }
}

#[test]
fn non_stream_failure_uses_redacted_openai_error_envelope() {
    let secret = "Authorization: Bearer non-stream-secret";
    let encoded = ResponsesCodec::encode_response(&[CanonicalEvent::Failed(
        GatewayError::upstream_auth(secret),
    )])
    .unwrap();
    let serialized = serde_json::to_string(&encoded).unwrap();

    assert_eq!(encoded, fixture_json("error/non_stream.json"));
    assert!(!serialized.contains(secret));
    assert!(!serialized.contains("non-stream-secret"));
    assert!(!serialized.contains("Bearer"));
}

#[test]
fn non_stream_response_matches_golden_and_preserves_safe_usage_extensions() {
    assert_eq!(
        ResponsesCodec::encode_response(&canonical_events()).unwrap(),
        fixture_json("response/complete.json")
    );
}

#[test]
fn response_extensions_cannot_override_standard_usage_fields() {
    let events = vec![
        CanonicalEvent::Created {
            response_id: "resp_1".to_owned(),
        },
        CanonicalEvent::Usage(Usage {
            input_tokens: 1,
            output_tokens: 2,
            cached_input_tokens: None,
            reasoning_tokens: None,
            extensions: BTreeMap::from([
                ("input_tokens".to_owned(), json!(999)),
                ("total_tokens".to_owned(), json!(999)),
                ("vendor".to_owned(), json!(true)),
            ]),
        }),
        CanonicalEvent::Completed,
    ];

    let encoded = ResponsesCodec::encode_response(&events).unwrap();
    assert_eq!(encoded["usage"]["input_tokens"], 1);
    assert_eq!(encoded["usage"]["total_tokens"], 3);
    assert_eq!(encoded["usage"]["vendor"], true);
}

#[test]
fn stream_events_cover_every_variant_and_match_golden_order() {
    let expected = fixture_json("stream/ordered.json");
    let mut codec = ResponsesCodec::default();
    let mut actual = Vec::new();

    for event in canonical_events() {
        let encoded = codec.encode_event(&event).unwrap();
        let mut decoder = SseDecoder::default();
        let frames = decoder.push(&encoded).unwrap();
        assert_eq!(frames.len(), 1);
        actual.push(json!({
            "event": frames[0].event,
            "data": serde_json::from_str::<Value>(&frames[0].data).unwrap()
        }));
    }

    assert_eq!(Value::Array(actual), expected);
}

#[test]
fn stream_encoder_rejects_events_after_completion() {
    let mut codec = ResponsesCodec::default();
    codec
        .encode_event(&CanonicalEvent::Created {
            response_id: "resp_1".to_owned(),
        })
        .unwrap();
    codec.encode_event(&CanonicalEvent::Completed).unwrap();

    assert_eq!(
        codec
            .encode_event(&CanonicalEvent::OutputTextDelta {
                item_id: "msg_1".to_owned(),
                delta: "late".to_owned(),
            })
            .unwrap_err(),
        GatewayError::invalid_request()
    );
}

#[test]
fn failed_event_is_redacted_and_matches_golden() {
    let secret = "Authorization: Bearer fixture-secret";
    let mut codec = ResponsesCodec::default();
    let encoded = codec
        .encode_event(&CanonicalEvent::Failed(GatewayError::upstream_auth(secret)))
        .unwrap();
    let encoded_text = String::from_utf8(encoded.to_vec()).unwrap();
    let mut decoder = SseDecoder::default();
    let frame = decoder.push(encoded_text.as_bytes()).unwrap().remove(0);

    assert_eq!(frame.event.as_deref(), Some("error"));
    assert_eq!(
        serde_json::from_str::<Value>(&frame.data).unwrap(),
        fixture_json("error/failed.json")
    );
    assert!(!encoded_text.contains(secret));
    assert!(!encoded_text.contains("fixture-secret"));
    assert!(!encoded_text.contains("Bearer"));
}

#[test]
fn non_stream_encoder_rejects_invalid_event_order() {
    let invalid_sequences = [
        vec![CanonicalEvent::Completed],
        vec![
            CanonicalEvent::Created {
                response_id: "resp_1".to_owned(),
            },
            CanonicalEvent::Completed,
            CanonicalEvent::Usage(Usage {
                input_tokens: 1,
                output_tokens: 1,
                cached_input_tokens: None,
                reasoning_tokens: None,
                extensions: BTreeMap::new(),
            }),
        ],
        vec![
            CanonicalEvent::Created {
                response_id: "resp_1".to_owned(),
            },
            CanonicalEvent::Failed(GatewayError::internal("secret")),
            CanonicalEvent::Completed,
        ],
    ];

    for events in invalid_sequences {
        assert_eq!(
            ResponsesCodec::encode_response(&events).unwrap_err(),
            GatewayError::invalid_request()
        );
    }
}
