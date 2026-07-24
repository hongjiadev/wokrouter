use std::collections::BTreeMap;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

use crate::{
    AnthropicCodec,
    canonical::{
        CanonicalRequest, GatewayError, InputItem, PublicModelId, ReasoningOptions, RequestId,
        ToolDefinition,
    },
};

pub(crate) const REQUEST_EXTENSION_KEY: &str = "anthropic.request";
const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;
const MAX_MESSAGES: usize = 4_096;
const MAX_CONTENT_BLOCKS: usize = 16_384;
const MAX_TOOLS: usize = 4_096;
const MAX_IDENTIFIER_BYTES: usize = 512;
const MAX_RETAINED_VALUE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy)]
pub(super) struct AnthropicInboundLimits {
    max_request_bytes: usize,
    max_messages: usize,
    max_content_blocks: usize,
    max_tools: usize,
    max_identifier_bytes: usize,
    max_retained_value_bytes: usize,
}

impl Default for AnthropicInboundLimits {
    fn default() -> Self {
        Self {
            max_request_bytes: MAX_REQUEST_BYTES,
            max_messages: MAX_MESSAGES,
            max_content_blocks: MAX_CONTENT_BLOCKS,
            max_tools: MAX_TOOLS,
            max_identifier_bytes: MAX_IDENTIFIER_BYTES,
            max_retained_value_bytes: MAX_RETAINED_VALUE_BYTES,
        }
    }
}

#[derive(Deserialize, Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<AnthropicContent>,
    #[serde(default)]
    tools: Vec<AnthropicToolWire>,
    #[serde(default)]
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<AnthropicThinkingConfig>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Deserialize, Serialize)]
struct AnthropicMessage {
    role: AnthropicRole,
    content: AnthropicContent,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum AnthropicRole {
    User,
    Assistant,
}

#[derive(Clone, Copy)]
enum ContentContext {
    System,
    User,
    Assistant,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum AnthropicContent {
    Text(String),
    Blocks(Vec<AnthropicContentBlock>),
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum AnthropicContentBlock {
    Known(AnthropicKnownContentBlock),
    Unknown(Value),
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type")]
enum AnthropicKnownContentBlock {
    #[serde(rename = "text")]
    Text {
        text: String,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
    #[serde(rename = "image")]
    Image {
        source: AnthropicSource,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
    #[serde(rename = "document")]
    Document {
        source: AnthropicSource,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
    #[serde(rename = "thinking")]
    Thinking {
        thinking: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
    #[serde(rename = "redacted_thinking")]
    RedactedThinking {
        data: String,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum AnthropicSource {
    Base64 {
        #[serde(rename = "type")]
        kind: Base64SourceKind,
        media_type: String,
        data: String,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
    Url {
        #[serde(rename = "type")]
        kind: UrlSourceKind,
        url: String,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
    Unknown(Value),
}

#[derive(Deserialize, Serialize)]
enum Base64SourceKind {
    #[serde(rename = "base64")]
    Base64,
}

#[derive(Deserialize, Serialize)]
enum UrlSourceKind {
    #[serde(rename = "url")]
    Url,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum AnthropicToolWire {
    Client(AnthropicTool),
    Unsupported(Value),
}

#[derive(Deserialize, Serialize)]
struct AnthropicTool {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    input_schema: Value,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Deserialize, Serialize)]
struct AnthropicThinkingConfig {
    #[serde(rename = "type")]
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    budget_tokens: Option<u64>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

impl AnthropicCodec {
    pub fn decode_message(
        request_id: RequestId,
        json: &[u8],
    ) -> Result<CanonicalRequest, GatewayError> {
        Self::decode_message_with_limits(request_id, json, AnthropicInboundLimits::default(), true)
    }

    pub(crate) fn decode_count_tokens(
        request_id: RequestId,
        json: &[u8],
    ) -> Result<CanonicalRequest, GatewayError> {
        Self::decode_message_with_limits(request_id, json, AnthropicInboundLimits::default(), false)
    }

    fn decode_message_with_limits(
        request_id: RequestId,
        json: &[u8],
        limits: AnthropicInboundLimits,
        require_max_tokens: bool,
    ) -> Result<CanonicalRequest, GatewayError> {
        if json.len() > limits.max_request_bytes {
            return Err(GatewayError::invalid_request());
        }
        let wire: AnthropicRequest =
            serde_json::from_slice(json).map_err(|_| GatewayError::invalid_request())?;
        validate_request(&wire, limits, require_max_tokens)?;

        // Retain the validated wire value itself so absent/default fields and block
        // boundaries survive an Anthropic-to-Anthropic route byte-semantically.
        let retained: Value =
            serde_json::from_slice(json).map_err(|_| GatewayError::invalid_request())?;
        limits.validate_value(&retained)?;

        let mut input = Vec::new();
        if let Some(system) = &wire.system {
            extract_content(system, &mut input, limits)?;
        }
        for message in &wire.messages {
            extract_content(&message.content, &mut input, limits)?;
        }
        let tools = wire
            .tools
            .iter()
            .filter_map(|tool| match tool {
                AnthropicToolWire::Client(tool) => Some(ToolDefinition {
                    name: tool.name.clone(),
                    description: tool.description.clone(),
                    input_schema: tool.input_schema.clone(),
                    extensions: tool.extra.clone(),
                }),
                AnthropicToolWire::Unsupported(_) => None,
            })
            .collect();
        let reasoning = wire.thinking.as_ref().map(|thinking| ReasoningOptions {
            effort: Some(thinking.kind.clone()),
            extensions: thinking
                .budget_tokens
                .map(|budget| BTreeMap::from([("budget_tokens".to_owned(), json!(budget))]))
                .unwrap_or_default(),
        });

        Ok(CanonicalRequest {
            request_id,
            model: PublicModelId::new(wire.model),
            thread_key: None,
            input,
            tools,
            stream: wire.stream,
            reasoning,
            extensions: BTreeMap::from([(REQUEST_EXTENSION_KEY.to_owned(), retained)]),
        })
    }
}

fn validate_request(
    wire: &AnthropicRequest,
    limits: AnthropicInboundLimits,
    require_max_tokens: bool,
) -> Result<(), GatewayError> {
    limits.validate_identifier(&wire.model)?;
    if wire.messages.is_empty()
        || wire.messages.len() > limits.max_messages
        || require_max_tokens && wire.max_tokens.is_none_or(|tokens| tokens == 0)
        || !require_max_tokens && wire.max_tokens.is_some()
    {
        return Err(GatewayError::invalid_request());
    }
    let mut block_count = 0_usize;
    if let Some(system) = &wire.system {
        block_count =
            block_count.saturating_add(validate_content(system, limits, ContentContext::System)?);
    }
    for message in &wire.messages {
        let context = match message.role {
            AnthropicRole::User => ContentContext::User,
            AnthropicRole::Assistant => ContentContext::Assistant,
        };
        block_count =
            block_count.saturating_add(validate_content(&message.content, limits, context)?);
    }
    if block_count > limits.max_content_blocks || wire.tools.len() > limits.max_tools {
        return Err(GatewayError::invalid_request());
    }
    for tool in &wire.tools {
        match tool {
            AnthropicToolWire::Client(tool) => {
                limits.validate_identifier(&tool.name)?;
                if !tool.input_schema.is_object() {
                    return Err(GatewayError::invalid_request());
                }
                limits.validate_value(&tool.input_schema)?;
            }
            AnthropicToolWire::Unsupported(_) => {
                return Err(GatewayError::unsupported_capability());
            }
        }
    }
    if let Some(thinking) = &wire.thinking {
        match thinking.kind.as_str() {
            "enabled" => {
                let Some(budget) = thinking.budget_tokens else {
                    return Err(GatewayError::invalid_request());
                };
                if budget < 1_024
                    || require_max_tokens
                        && wire
                            .max_tokens
                            .is_some_and(|max_tokens| budget >= max_tokens)
                {
                    return Err(GatewayError::invalid_request());
                }
            }
            "adaptive" | "disabled" if thinking.budget_tokens.is_none() => {}
            "adaptive" | "disabled" => return Err(GatewayError::invalid_request()),
            _ => return Err(GatewayError::unsupported_capability()),
        }
    }
    Ok(())
}

fn validate_content(
    content: &AnthropicContent,
    limits: AnthropicInboundLimits,
    context: ContentContext,
) -> Result<usize, GatewayError> {
    match content {
        AnthropicContent::Text(_) => Ok(1),
        AnthropicContent::Blocks(blocks) => {
            if blocks.is_empty() {
                return Err(GatewayError::invalid_request());
            }
            for block in blocks {
                match block {
                    AnthropicContentBlock::Known(AnthropicKnownContentBlock::Text { .. }) => {}
                    AnthropicContentBlock::Known(AnthropicKnownContentBlock::Image {
                        source,
                        ..
                    }) if matches!(context, ContentContext::User) => {
                        validate_source(source, false, limits)?
                    }
                    AnthropicContentBlock::Known(AnthropicKnownContentBlock::Document {
                        source,
                        ..
                    }) if matches!(context, ContentContext::User) => {
                        validate_source(source, true, limits)?
                    }
                    AnthropicContentBlock::Known(AnthropicKnownContentBlock::ToolUse {
                        id,
                        name,
                        input,
                        ..
                    }) if matches!(context, ContentContext::Assistant) => {
                        limits.validate_identifier(id)?;
                        limits.validate_identifier(name)?;
                        if !input.is_object() {
                            return Err(GatewayError::invalid_request());
                        }
                        limits.validate_value(input)?;
                    }
                    AnthropicContentBlock::Known(AnthropicKnownContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        ..
                    }) if matches!(context, ContentContext::User) => {
                        limits.validate_identifier(tool_use_id)?;
                        if let Some(content) = content {
                            limits.validate_value(content)?;
                        }
                    }
                    AnthropicContentBlock::Known(AnthropicKnownContentBlock::Thinking {
                        signature,
                        ..
                    }) if matches!(context, ContentContext::Assistant) => {
                        if let Some(signature) = signature {
                            limits.validate_identifier(signature)?;
                        }
                    }
                    AnthropicContentBlock::Known(
                        AnthropicKnownContentBlock::RedactedThinking { data, .. },
                    ) if matches!(context, ContentContext::Assistant) => {
                        limits.validate_identifier(data)?
                    }
                    AnthropicContentBlock::Unknown(_) => {
                        return Err(GatewayError::unsupported_capability());
                    }
                    _ => return Err(GatewayError::invalid_request()),
                }
            }
            Ok(blocks.len())
        }
    }
}

fn validate_source(
    source: &AnthropicSource,
    document: bool,
    limits: AnthropicInboundLimits,
) -> Result<(), GatewayError> {
    match source {
        AnthropicSource::Base64 {
            media_type, data, ..
        } => {
            let allowed = if document {
                media_type.eq_ignore_ascii_case("application/pdf")
                    || media_type.eq_ignore_ascii_case("text/plain")
            } else {
                matches!(
                    media_type.to_ascii_lowercase().as_str(),
                    "image/jpeg" | "image/png" | "image/gif" | "image/webp"
                )
            };
            if !allowed || data.is_empty() || STANDARD.decode(data.as_bytes()).is_err() {
                return Err(GatewayError::invalid_request());
            }
        }
        AnthropicSource::Url { url, .. } => {
            let parsed = Url::parse(url).map_err(|_| GatewayError::invalid_request())?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err(GatewayError::invalid_request());
            }
        }
        AnthropicSource::Unknown(_) => return Err(GatewayError::unsupported_capability()),
    }
    limits
        .validate_value(&serde_json::to_value(source).map_err(|_| GatewayError::invalid_request())?)
}

fn extract_content(
    content: &AnthropicContent,
    input: &mut Vec<InputItem>,
    limits: AnthropicInboundLimits,
) -> Result<(), GatewayError> {
    match content {
        AnthropicContent::Text(text) => input.push(InputItem::Text { text: text.clone() }),
        AnthropicContent::Blocks(blocks) => {
            for block in blocks {
                match block {
                    AnthropicContentBlock::Known(AnthropicKnownContentBlock::Text {
                        text, ..
                    }) => input.push(InputItem::Text { text: text.clone() }),
                    AnthropicContentBlock::Known(AnthropicKnownContentBlock::Image {
                        source,
                        ..
                    }) => input.push(InputItem::ImageUrl {
                        url: source_url(source)?,
                        detail: None,
                    }),
                    AnthropicContentBlock::Known(AnthropicKnownContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        ..
                    }) => input.push(InputItem::ToolResult {
                        call_id: tool_use_id.clone(),
                        output: content.clone().unwrap_or_else(|| json!("")),
                    }),
                    _ => {}
                }
            }
        }
    }
    if input.len() > limits.max_content_blocks {
        Err(GatewayError::invalid_request())
    } else {
        Ok(())
    }
}

fn source_url(source: &AnthropicSource) -> Result<Url, GatewayError> {
    match source {
        AnthropicSource::Base64 {
            media_type, data, ..
        } => Url::parse(&format!("data:{media_type};base64,{data}"))
            .map_err(|_| GatewayError::invalid_request()),
        AnthropicSource::Url { url, .. } => {
            Url::parse(url).map_err(|_| GatewayError::invalid_request())
        }
        AnthropicSource::Unknown(_) => Err(GatewayError::unsupported_capability()),
    }
}

impl AnthropicInboundLimits {
    fn validate_identifier(&self, value: &str) -> Result<(), GatewayError> {
        if value.is_empty() || value.len() > self.max_identifier_bytes {
            Err(GatewayError::invalid_request())
        } else {
            Ok(())
        }
    }

    fn validate_value(&self, value: &Value) -> Result<(), GatewayError> {
        if serde_json::to_vec(value)
            .is_ok_and(|encoded| encoded.len() <= self.max_retained_value_bytes)
        {
            Ok(())
        } else {
            Err(GatewayError::invalid_request())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_limits() -> AnthropicInboundLimits {
        AnthropicInboundLimits {
            max_request_bytes: 1_024,
            max_messages: 1,
            max_content_blocks: 2,
            max_tools: 1,
            max_identifier_bytes: 8,
            max_retained_value_bytes: 512,
        }
    }

    fn decode(body: Value) -> Result<CanonicalRequest, GatewayError> {
        AnthropicCodec::decode_message_with_limits(
            RequestId::new("req"),
            &serde_json::to_vec(&body).unwrap(),
            tiny_limits(),
            true,
        )
    }

    #[test]
    fn private_limits_bound_every_retained_request_collection_and_value() {
        for body in [
            json!({
                "model": "123456789",
                "max_tokens": 1,
                "messages": [{"role": "user", "content": "x"}],
            }),
            json!({
                "model": "model",
                "max_tokens": 1,
                "messages": [
                    {"role": "user", "content": "x"},
                    {"role": "assistant", "content": "y"}
                ],
            }),
            json!({
                "model": "model",
                "max_tokens": 1,
                "messages": [{"role": "user", "content": [
                    {"type": "text", "text": "1"},
                    {"type": "text", "text": "2"},
                    {"type": "text", "text": "3"}
                ]}],
            }),
            json!({
                "model": "model",
                "max_tokens": 1,
                "messages": [{"role": "user", "content": "x"}],
                "tools": [
                    {"name": "one", "input_schema": {"type": "object"}},
                    {"name": "two", "input_schema": {"type": "object"}}
                ],
            }),
        ] {
            assert_eq!(decode(body).unwrap_err(), GatewayError::invalid_request());
        }

        let oversized = json!({
            "model": "model",
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "x".repeat(600)}],
        });
        assert_eq!(
            decode(oversized).unwrap_err(),
            GatewayError::invalid_request()
        );
    }

    #[test]
    fn private_request_body_limit_is_independent_from_retained_value_limit() {
        let mut limits = tiny_limits();
        limits.max_request_bytes = 32;
        limits.max_retained_value_bytes = 1_024;
        let body = serde_json::to_vec(&json!({
            "model": "model",
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "safe"}],
        }))
        .unwrap();
        assert_eq!(
            AnthropicCodec::decode_message_with_limits(RequestId::new("req"), &body, limits, true,)
                .unwrap_err(),
            GatewayError::invalid_request()
        );
    }
}
