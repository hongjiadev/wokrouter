use std::{collections::BTreeMap, fs, path::PathBuf};

use url::Url;
use wokrouter_protocols::{
    AzureAdapter, AzureConfig, CursorAdapter, CursorConfig, GeminiAdapter, GeminiConfig,
    UpstreamLimits,
    canonical::{
        CanonicalEvent, CanonicalRequest, InputItem, PublicModelId, RequestId, ToolDefinition,
    },
};

fn fixture_bytes(relative: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/protocols")
        .join(relative);
    fs::read(path).unwrap()
}

fn fixture_hex(relative: &str) -> Vec<u8> {
    let source = String::from_utf8(fixture_bytes(relative)).unwrap();
    source
        .lines()
        .filter_map(|line| line.split('#').next())
        .flat_map(str::split_whitespace)
        .map(|byte| u8::from_str_radix(byte, 16).unwrap())
        .collect()
}

fn request(model: &str, stream: bool) -> CanonicalRequest {
    CanonicalRequest {
        request_id: RequestId::new("req_fixture"),
        model: PublicModelId::new(model),
        thread_key: None,
        input: vec![InputItem::Text {
            text: "Use the weather tool.".to_owned(),
        }],
        tools: vec![ToolDefinition {
            name: "weather".to_owned(),
            description: Some("Get weather".to_owned()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"city": {"type": "string"}},
                "required": ["city"]
            }),
            extensions: BTreeMap::new(),
        }],
        stream,
        reasoning: None,
        extensions: BTreeMap::new(),
    }
}

#[test]
fn gemini_tool_call_stream_becomes_canonical_deltas() {
    let adapter = GeminiAdapter::new(
        GeminiConfig::new(
            Url::parse("https://generativelanguage.googleapis.com/").unwrap(),
            "fixture-secret",
        )
        .unwrap(),
        UpstreamLimits::default(),
    );
    let outbound = adapter
        .build_request(&request("gemini-2.5-flash", true))
        .unwrap();
    assert_eq!(
        outbound.url.as_str(),
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
    );
    assert!(!outbound.url.as_str().contains("fixture-secret"));

    let mut decoder = adapter.stream_decoder(RequestId::new("req_fixture"));
    let body = fixture_bytes("gemini/tool_stream.sse");
    let split = body.len() / 2;
    let mut events = decoder.push(&body[..split]).unwrap();
    events.extend(decoder.push(&body[split..]).unwrap());
    events.extend(decoder.finish().unwrap());

    assert!(events.iter().any(|event| matches!(
        event,
        CanonicalEvent::ToolCallDelta { call_id, name, delta, .. }
            if call_id == "call_weather"
                && name == "weather"
                && delta == "{\"city\":\"Paris\"}"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        CanonicalEvent::Usage(usage)
            if usage.input_tokens == 9 && usage.output_tokens == 4
    )));
    assert_eq!(events.last(), Some(&CanonicalEvent::Completed));
}

#[test]
fn gemini_rejects_endpoint_escape_and_bounds_stream_frames() {
    assert_eq!(
        GeminiConfig::new(Url::parse("file:///tmp/provider/").unwrap(), "secret")
            .unwrap_err()
            .code(),
        "invalid_request"
    );

    let adapter = GeminiAdapter::new(
        GeminiConfig::new(Url::parse("https://example.test/root/").unwrap(), "secret").unwrap(),
        UpstreamLimits {
            max_stream_frame_bytes: 32,
            ..UpstreamLimits::default()
        },
    );
    assert_eq!(
        adapter
            .stream_decoder(RequestId::new("req_bound"))
            .push(
                b"data: {\"text\":\"this frame is deliberately larger than thirty-two bytes\"}\n\n"
            )
            .unwrap_err()
            .code(),
        "invalid_request"
    );
}

#[test]
fn azure_request_response_and_url_validation_are_canonical() {
    let adapter = AzureAdapter::new(
        AzureConfig::new(
            Url::parse("https://fixture.openai.azure.com/").unwrap(),
            "deployment-a",
            "2024-10-21",
            "fixture-azure-key",
        )
        .unwrap(),
        UpstreamLimits::default(),
    );
    let outbound = adapter
        .build_request(&request("public-alias", false))
        .unwrap();
    assert_eq!(
        outbound.url.as_str(),
        "https://fixture.openai.azure.com/openai/deployments/deployment-a/chat/completions?api-version=2024-10-21"
    );
    assert_eq!(
        outbound.headers.get("api-key").map(String::as_str),
        Some("fixture-azure-key")
    );
    assert!(!outbound.url.as_str().contains("fixture-azure-key"));

    let events = adapter
        .decode_response(
            RequestId::new("req_azure"),
            &fixture_bytes("azure/response/tool.json"),
        )
        .unwrap();
    assert!(events.iter().any(|event| matches!(
        event,
        CanonicalEvent::ToolCallDelta { call_id, name, delta, .. }
            if call_id == "call_azure" && name == "weather" && delta == "{\"city\":\"Paris\"}"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        CanonicalEvent::Usage(usage)
            if usage.input_tokens == 7 && usage.output_tokens == 3
    )));

    for deployment in ["", "../escape", "bad/name", "bad?query"] {
        assert_eq!(
            AzureConfig::new(
                Url::parse("https://fixture.openai.azure.com/").unwrap(),
                deployment,
                "2024-10-21",
                "secret",
            )
            .unwrap_err()
            .code(),
            "invalid_request"
        );
    }
    for deployment in [".", ".."] {
        assert_eq!(
            AzureConfig::new(
                Url::parse("https://fixture.openai.azure.com/").unwrap(),
                deployment,
                "2024-10-21",
                "secret",
            )
            .unwrap_err()
            .code(),
            "invalid_request"
        );
    }
    for api_version in ["", "v1", "2024-10-21?other=true", "2024-13-01"] {
        assert_eq!(
            AzureConfig::new(
                Url::parse("https://fixture.openai.azure.com/").unwrap(),
                "safe-deployment",
                api_version,
                "secret",
            )
            .unwrap_err()
            .code(),
            "invalid_request"
        );
    }
}

#[test]
fn azure_stream_keeps_partial_tool_arguments_opaque_until_wire_delta() {
    let adapter = AzureAdapter::new(
        AzureConfig::new(
            Url::parse("https://fixture.openai.azure.com/").unwrap(),
            "deployment-a",
            "2024-10-21",
            "secret",
        )
        .unwrap(),
        UpstreamLimits::default(),
    );
    let mut decoder = adapter.stream_decoder(RequestId::new("req_azure_stream"));
    let events = decoder
        .push(&fixture_bytes("azure/stream/tool.sse"))
        .unwrap();
    let tool_deltas = events
        .iter()
        .filter_map(|event| match event {
            CanonicalEvent::ToolCallDelta { delta, .. } => Some(delta.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(tool_deltas, ["{\"city\":", "\"Paris\"}"]);
    assert_eq!(
        decoder.finish().unwrap().last(),
        Some(&CanonicalEvent::Completed)
    );
}

#[test]
fn cursor_is_opt_in_maps_events_and_never_enables_native_execution() {
    let disabled = CursorAdapter::new(
        CursorConfig::new(Url::parse("https://api2.cursor.sh/").unwrap(), false).unwrap(),
        UpstreamLimits::default(),
    );
    assert_eq!(
        disabled
            .build_request(&request("cursor/composer-2.5", true))
            .unwrap_err()
            .code(),
        "unsupported_capability"
    );

    let enabled = CursorAdapter::new(
        CursorConfig::new(Url::parse("https://api2.cursor.sh/").unwrap(), true).unwrap(),
        UpstreamLimits::default(),
    );
    let outbound = enabled
        .build_request(&request("cursor/composer-2.5", true))
        .unwrap();
    assert_eq!(
        outbound.headers.get("content-type").map(String::as_str),
        Some("application/connect+proto")
    );
    assert_eq!(outbound.body[0], 0);
    assert_eq!(
        u32::from_be_bytes(outbound.body[1..5].try_into().unwrap()) as usize,
        outbound.body.len() - 5
    );
    assert!(!outbound.body.starts_with(b"{"));
    assert!(
        outbound
            .body
            .windows("composer-2.5".len())
            .any(|window| window == b"composer-2.5")
    );

    let mut decoder = enabled.stream_decoder(RequestId::new("req_cursor"));
    let fixture = fixture_hex("cursor/stream/tool.connect.hex");
    let mut events = decoder.push(&fixture[..7]).unwrap();
    events.extend(decoder.push(&fixture[7..]).unwrap());
    events.extend(decoder.finish().unwrap());
    assert!(events.iter().any(|event| matches!(
        event,
        CanonicalEvent::ReasoningDelta { delta, .. } if delta == "Checking."
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        CanonicalEvent::ToolCallDelta { call_id, name, delta, .. }
            if call_id == "call_cursor" && name == "weather" && delta == "{\"city\":\"Paris\"}"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        CanonicalEvent::Usage(usage) if usage.output_tokens == 3
    )));
    assert_eq!(events.last(), Some(&CanonicalEvent::Completed));
}

#[test]
fn cursor_request_matches_independently_encoded_protobuf_fixture() {
    let adapter = CursorAdapter::new(
        CursorConfig::new(Url::parse("https://api2.cursor.sh/").unwrap(), true).unwrap(),
        UpstreamLimits::default(),
    );
    let request = CanonicalRequest {
        request_id: RequestId::new("req_wire"),
        model: PublicModelId::new("cursor/composer-2.5"),
        thread_key: Some(wokrouter_protocols::canonical::ThreadKey::new(
            "thread_wire",
        )),
        input: vec![InputItem::Text {
            text: "hi".to_owned(),
        }],
        tools: Vec::new(),
        stream: true,
        reasoning: None,
        extensions: BTreeMap::new(),
    };

    assert_eq!(
        adapter.build_request(&request).unwrap().body,
        fixture_hex("cursor/request/run.connect.hex")
    );
}

#[test]
fn cursor_executor_hook_without_executor_is_typed_and_redacted() {
    let adapter = CursorAdapter::new(
        CursorConfig::new(
            Url::parse("https://internal.example.test/secret-base/").unwrap(),
            true,
        )
        .unwrap(),
        UpstreamLimits::default(),
    );
    let mut decoder = adapter.stream_decoder(RequestId::new("req_cursor_exec"));
    let error = decoder
        .push(&fixture_hex("cursor/stream/exec.connect.hex"))
        .unwrap_err();
    assert_eq!(error.code(), "no_executor");
    let serialized = serde_json::to_string(&error).unwrap();
    assert!(!serialized.contains("internal.example.test"));
}

#[test]
fn generated_response_and_cursor_conversation_ids_are_individually_bounded() {
    let gemini = GeminiAdapter::new(
        GeminiConfig::new(Url::parse("https://example.test/").unwrap(), "secret").unwrap(),
        UpstreamLimits {
            max_identifier_bytes: 16,
            ..UpstreamLimits::default()
        },
    );
    assert_eq!(
        gemini
            .decode_response(RequestId::new("x".repeat(32)), br#"{"candidates":[]}"#)
            .unwrap_err()
            .code(),
        "invalid_request"
    );

    let cursor = CursorAdapter::new(
        CursorConfig::new(Url::parse("https://example.test/").unwrap(), true).unwrap(),
        UpstreamLimits {
            max_identifier_bytes: 16,
            ..UpstreamLimits::default()
        },
    );
    let mut request = request("cursor/auto", true);
    request.thread_key = Some(wokrouter_protocols::canonical::ThreadKey::new(
        "conversation-key-that-is-too-long",
    ));
    assert_eq!(
        cursor.build_request(&request).unwrap_err().code(),
        "invalid_request"
    );
}

#[test]
fn cursor_connect_decoder_enforces_frame_aggregate_event_and_tool_limits() {
    let fixture = fixture_hex("cursor/stream/tool.connect.hex");
    for limits in [
        UpstreamLimits {
            max_stream_frame_bytes: 8,
            ..UpstreamLimits::default()
        },
        UpstreamLimits {
            max_response_body_bytes: fixture.len() - 1,
            ..UpstreamLimits::default()
        },
        UpstreamLimits {
            max_events: 1,
            ..UpstreamLimits::default()
        },
        UpstreamLimits {
            max_tool_argument_bytes: 8,
            ..UpstreamLimits::default()
        },
    ] {
        let adapter = CursorAdapter::new(
            CursorConfig::new(Url::parse("https://api2.cursor.sh/").unwrap(), true).unwrap(),
            limits,
        );
        let mut decoder = adapter.stream_decoder(RequestId::new("req_limits"));
        assert_eq!(
            decoder.push(&fixture).unwrap_err().code(),
            "invalid_request"
        );
    }
}

#[test]
fn stream_event_limits_are_aggregate_across_push_and_finish_calls() {
    let limits = UpstreamLimits {
        max_events: 2,
        ..UpstreamLimits::default()
    };
    let gemini = GeminiAdapter::new(
        GeminiConfig::new(Url::parse("https://example.test/").unwrap(), "secret").unwrap(),
        limits,
    );
    let mut gemini_decoder = gemini.stream_decoder(RequestId::new("req_events"));
    assert_eq!(
        gemini_decoder
            .push(b"data: {\"candidates\":[]}\n\n")
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        gemini_decoder
            .push(b"data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"x\"}]}}]}\n\n")
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        gemini_decoder.finish().unwrap_err().code(),
        "invalid_request"
    );

    let azure = AzureAdapter::new(
        AzureConfig::new(
            Url::parse("https://example.test/").unwrap(),
            "deployment",
            "2024-10-21",
            "secret",
        )
        .unwrap(),
        limits,
    );
    let mut azure_decoder = azure.stream_decoder(RequestId::new("req_events"));
    assert_eq!(
        azure_decoder
            .push(b"data: {\"id\":\"az\",\"choices\":[]}\n\n")
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        azure_decoder
            .push(b"data: {\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n\n")
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        azure_decoder.finish().unwrap_err().code(),
        "invalid_request"
    );

    let cursor = CursorAdapter::new(
        CursorConfig::new(Url::parse("https://api2.cursor.sh/").unwrap(), true).unwrap(),
        limits,
    );
    let fixture = fixture_hex("cursor/stream/tool.connect.hex");
    let first_frame_end = 16;
    let second_frame_end = first_frame_end + 20;
    let mut cursor_decoder = cursor.stream_decoder(RequestId::new("req_events"));
    assert_eq!(
        cursor_decoder
            .push(&fixture[..first_frame_end])
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        cursor_decoder
            .push(&fixture[first_frame_end..second_frame_end])
            .unwrap_err()
            .code(),
        "invalid_request"
    );
}

#[test]
fn cursor_connect_terminal_states_fail_closed() {
    let adapter = CursorAdapter::new(
        CursorConfig::new(Url::parse("https://api2.cursor.sh/").unwrap(), true).unwrap(),
        UpstreamLimits::default(),
    );
    let fixture = fixture_hex("cursor/stream/tool.connect.hex");
    let first_frame_end = 16;

    let mut truncated = adapter.stream_decoder(RequestId::new("req_truncated"));
    truncated.push(&fixture[..first_frame_end]).unwrap();
    assert_eq!(truncated.finish().unwrap_err().code(), "invalid_request");

    let mut ended = adapter.stream_decoder(RequestId::new("req_ended"));
    ended.push(&fixture).unwrap();
    assert_eq!(
        ended.push(&fixture[..first_frame_end]).unwrap_err().code(),
        "invalid_request"
    );

    let mut rejected_exec = adapter.stream_decoder(RequestId::new("req_exec"));
    assert_eq!(
        rejected_exec
            .push(&fixture_hex("cursor/stream/exec.connect.hex"))
            .unwrap_err()
            .code(),
        "no_executor"
    );
    assert_eq!(
        rejected_exec
            .push(&fixture[..first_frame_end])
            .unwrap_err()
            .code(),
        "invalid_request"
    );
    assert_eq!(
        rejected_exec.finish().unwrap_err().code(),
        "invalid_request"
    );
}

#[test]
fn base_urls_must_have_explicit_directory_semantics() {
    for base in [
        "https://example.test/root",
        "https://example.test/root?query=true",
        "https://user@example.test/",
    ] {
        assert_eq!(
            GeminiConfig::new(Url::parse(base).unwrap(), "secret")
                .unwrap_err()
                .code(),
            "invalid_request"
        );
    }
}

#[test]
fn response_body_limit_fails_without_retaining_the_body() {
    let adapter = AzureAdapter::new(
        AzureConfig::new(
            Url::parse("https://fixture.openai.azure.com/").unwrap(),
            "deployment-a",
            "2024-10-21",
            "secret",
        )
        .unwrap(),
        UpstreamLimits {
            max_response_body_bytes: 16,
            ..UpstreamLimits::default()
        },
    );
    let secret_body = br#"{"secret":"must-not-be-retained"}"#;
    let error = adapter
        .decode_response(RequestId::new("req_bound"), secret_body)
        .unwrap_err();
    assert_eq!(error.code(), "invalid_request");
    assert!(!format!("{error:?}").contains("must-not-be-retained"));
}

#[test]
fn upstream_http_errors_are_typed_without_body_or_endpoint_diagnostics() {
    let gemini = GeminiAdapter::new(
        GeminiConfig::new(
            Url::parse("https://internal-gemini.example.test/").unwrap(),
            "gemini-secret",
        )
        .unwrap(),
        UpstreamLimits::default(),
    );
    let azure = AzureAdapter::new(
        AzureConfig::new(
            Url::parse("https://internal-azure.example.test/").unwrap(),
            "private-deployment",
            "2024-10-21",
            "azure-secret",
        )
        .unwrap(),
        UpstreamLimits::default(),
    );
    let cursor = CursorAdapter::new(
        CursorConfig::new(
            Url::parse("https://internal-cursor.example.test/").unwrap(),
            true,
        )
        .unwrap(),
        UpstreamLimits::default(),
    );

    assert_eq!(gemini.decode_http_error(401, None).code(), "upstream_auth");
    assert_eq!(
        azure.decode_http_error(429, Some("12")).code(),
        "rate_limited"
    );
    assert_eq!(cursor.decode_http_error(503, None).code(), "upstream_error");
    for error in [
        gemini.decode_http_error(401, None),
        azure.decode_http_error(400, None),
        cursor.decode_http_error(503, None),
    ] {
        let debug = format!("{error:?}");
        for private in [
            "gemini-secret",
            "azure-secret",
            "private-deployment",
            "internal-gemini",
            "internal-azure",
            "internal-cursor",
        ] {
            assert!(!debug.contains(private));
        }
    }
}
