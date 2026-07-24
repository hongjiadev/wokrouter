use std::collections::BTreeMap;

use bytes::Bytes;
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::{
    canonical::{CanonicalEvent, GatewayError, PublicModelId, Usage},
    stream::encode_sse,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChatFinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    FunctionCall,
}

impl ChatFinishReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Length => "length",
            Self::ToolCalls => "tool_calls",
            Self::ContentFilter => "content_filter",
            Self::FunctionCall => "function_call",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatEncodeContext {
    pub model: PublicModelId,
    pub created: u64,
    pub response: ChatResponseTemplate,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatResponseTemplate {
    pub choice_index: u64,
    pub finish_reason: ChatFinishReason,
    pub logprobs: Option<Value>,
    pub include_usage: bool,
    pub extra: BTreeMap<String, Value>,
}

pub struct ChatCodec {
    context: ChatEncodeContext,
    terminal: bool,
    response_id: Option<String>,
    text: String,
    identities: BTreeMap<String, OutputIdentity>,
    tools: Vec<ChatToolOutput>,
    usage: Option<Value>,
    response: Option<Value>,
}

enum OutputIdentity {
    Text,
    Tool(usize),
}

struct ChatToolOutput {
    call_id: String,
    name: String,
    arguments: String,
}

#[derive(Serialize)]
struct ChatCompletionWire<'a> {
    id: &'a str,
    object: &'static str,
    created: u64,
    model: &'a str,
    choices: Vec<Value>,
    usage: Value,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Serialize)]
struct ChatChunkWire<'a> {
    id: &'a str,
    object: &'static str,
    created: u64,
    model: &'a str,
    choices: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<Value>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Serialize)]
struct ChatErrorEnvelope<'a> {
    error: ChatErrorWire<'a>,
}

#[derive(Serialize)]
struct ChatErrorWire<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    code: &'static str,
    message: &'a str,
}

enum Transition {
    None,
    Chunks(Vec<Value>),
    Failed(Value),
}

impl ChatCodec {
    pub fn new(context: ChatEncodeContext) -> Self {
        Self {
            context,
            terminal: false,
            response_id: None,
            text: String::new(),
            identities: BTreeMap::new(),
            tools: Vec::new(),
            usage: None,
            response: None,
        }
    }

    pub fn encode_response(
        context: ChatEncodeContext,
        events: &[CanonicalEvent],
    ) -> Result<Value, GatewayError> {
        let mut codec = Self::new(context);
        for event in events {
            codec.transition(event)?;
        }
        if !codec.terminal {
            return Err(GatewayError::invalid_request());
        }
        codec.response.ok_or_else(GatewayError::invalid_request)
    }

    pub fn encode_chunk(&mut self, event: &CanonicalEvent) -> Result<Bytes, GatewayError> {
        let transition = self.transition(event)?;
        let mut encoded = Vec::new();
        match transition {
            Transition::None => {}
            Transition::Chunks(chunks) => {
                for chunk in chunks {
                    encoded.extend_from_slice(&encode_sse(None, &chunk));
                }
            }
            Transition::Failed(error) => {
                encoded.extend_from_slice(&encode_sse(None, &error));
            }
        }
        if self.terminal {
            encoded.extend_from_slice(b"data: [DONE]\n\n");
        }
        Ok(Bytes::from(encoded))
    }

    fn transition(&mut self, event: &CanonicalEvent) -> Result<Transition, GatewayError> {
        if self.terminal {
            return Err(GatewayError::invalid_request());
        }

        match event {
            CanonicalEvent::Created { response_id } => self.created(response_id),
            CanonicalEvent::OutputTextDelta { item_id, delta } => self.text_delta(item_id, delta),
            CanonicalEvent::ReasoningDelta { .. } => Err(GatewayError::unsupported_capability()),
            CanonicalEvent::ToolCallDelta {
                item_id,
                call_id,
                name,
                delta,
            } => self.tool_delta(item_id, call_id, name, delta),
            CanonicalEvent::Usage(usage) => self.usage(usage),
            CanonicalEvent::Completed => self.completed(),
            CanonicalEvent::Failed(error) => Ok(self.failed(error)),
        }
    }

    fn created(&mut self, response_id: &str) -> Result<Transition, GatewayError> {
        if self.response_id.is_some() || response_id.is_empty() {
            return Err(GatewayError::invalid_request());
        }
        self.response_id = Some(response_id.to_owned());
        let choice = self.choice(json!({"role": "assistant"}), Value::Null);
        Ok(Transition::Chunks(vec![
            self.chunk(vec![choice], Value::Null),
        ]))
    }

    fn text_delta(&mut self, item_id: &str, delta: &str) -> Result<Transition, GatewayError> {
        self.require_delta_allowed()?;
        if item_id.is_empty() {
            return Err(GatewayError::invalid_request());
        }
        match self.identities.get(item_id) {
            Some(OutputIdentity::Text) => {}
            Some(OutputIdentity::Tool(_)) => return Err(GatewayError::invalid_request()),
            None => {
                self.identities
                    .insert(item_id.to_owned(), OutputIdentity::Text);
            }
        }
        self.text.push_str(delta);
        let choice = self.choice(json!({"content": delta}), Value::Null);
        Ok(Transition::Chunks(vec![
            self.chunk(vec![choice], Value::Null),
        ]))
    }

    fn tool_delta(
        &mut self,
        item_id: &str,
        call_id: &str,
        name: &str,
        delta: &str,
    ) -> Result<Transition, GatewayError> {
        self.require_delta_allowed()?;
        if item_id.is_empty() || call_id.is_empty() || name.is_empty() {
            return Err(GatewayError::invalid_request());
        }

        let (tool_index, is_new) = match self.identities.get(item_id) {
            Some(OutputIdentity::Text) => return Err(GatewayError::invalid_request()),
            Some(OutputIdentity::Tool(tool_index)) => {
                let tool = &self.tools[*tool_index];
                if tool.call_id != call_id || tool.name != name {
                    return Err(GatewayError::invalid_request());
                }
                (*tool_index, false)
            }
            None => {
                if self.tools.iter().any(|tool| tool.call_id == call_id) {
                    return Err(GatewayError::invalid_request());
                }
                let tool_index = self.tools.len();
                self.tools.push(ChatToolOutput {
                    call_id: call_id.to_owned(),
                    name: name.to_owned(),
                    arguments: String::new(),
                });
                self.identities
                    .insert(item_id.to_owned(), OutputIdentity::Tool(tool_index));
                (tool_index, true)
            }
        };
        self.tools[tool_index].arguments.push_str(delta);

        let tool_delta = if is_new {
            json!({
                "index": tool_index,
                "id": call_id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": delta,
                },
            })
        } else {
            json!({
                "index": tool_index,
                "function": {
                    "arguments": delta,
                },
            })
        };
        let choice = self.choice(json!({"tool_calls": [tool_delta]}), Value::Null);
        Ok(Transition::Chunks(vec![
            self.chunk(vec![choice], Value::Null),
        ]))
    }

    fn usage(&mut self, usage: &Usage) -> Result<Transition, GatewayError> {
        self.require_created()?;
        if self.usage.is_some() {
            return Err(GatewayError::invalid_request());
        }
        self.usage = Some(usage_value(usage));
        Ok(Transition::None)
    }

    fn completed(&mut self) -> Result<Transition, GatewayError> {
        self.require_created()?;
        let usage = self
            .usage
            .clone()
            .ok_or_else(GatewayError::invalid_request)?;
        self.response = Some(self.response_value(usage.clone()));

        let finish_reason = json!(self.context.response.finish_reason.as_str());
        let finish_choice = self.choice(json!({}), finish_reason);
        let mut chunks = vec![self.chunk(vec![finish_choice], Value::Null)];
        if self.context.response.include_usage {
            chunks.push(self.chunk(Vec::new(), usage));
        }
        self.terminal = true;
        Ok(Transition::Chunks(chunks))
    }

    fn failed(&mut self, error: &GatewayError) -> Transition {
        let envelope = error_envelope(error);
        self.response = Some(envelope.clone());
        self.terminal = true;
        Transition::Failed(envelope)
    }

    fn require_created(&self) -> Result<(), GatewayError> {
        if self.response_id.is_some() {
            Ok(())
        } else {
            Err(GatewayError::invalid_request())
        }
    }

    fn require_delta_allowed(&self) -> Result<(), GatewayError> {
        self.require_created()?;
        if self.usage.is_none() {
            Ok(())
        } else {
            Err(GatewayError::invalid_request())
        }
    }

    fn choice(&self, delta: Value, finish_reason: Value) -> Value {
        json!({
            "index": self.context.response.choice_index,
            "delta": delta,
            "logprobs": self.context.response.logprobs,
            "finish_reason": finish_reason,
        })
    }

    fn chunk(&self, choices: Vec<Value>, usage: Value) -> Value {
        serde_json::to_value(ChatChunkWire {
            id: self.response_id.as_deref().unwrap_or_default(),
            object: "chat.completion.chunk",
            created: self.context.created,
            model: self.context.model.as_str(),
            choices,
            usage: self.context.response.include_usage.then_some(usage),
            extra: filtered_extra(
                &self.context.response.extra,
                &["id", "object", "created", "model", "choices", "usage"],
            ),
        })
        .expect("serializing a Chat Completions chunk cannot fail")
    }

    fn response_value(&self, usage: Value) -> Value {
        let mut message = Map::from_iter([
            ("role".to_owned(), json!("assistant")),
            (
                "content".to_owned(),
                if self.text.is_empty() {
                    Value::Null
                } else {
                    json!(self.text)
                },
            ),
        ]);
        if !self.tools.is_empty() {
            message.insert(
                "tool_calls".to_owned(),
                Value::Array(
                    self.tools
                        .iter()
                        .map(|tool| {
                            json!({
                                "id": tool.call_id,
                                "type": "function",
                                "function": {
                                    "name": tool.name,
                                    "arguments": tool.arguments,
                                },
                            })
                        })
                        .collect(),
                ),
            );
        }

        let choice = json!({
            "index": self.context.response.choice_index,
            "message": Value::Object(message),
            "logprobs": self.context.response.logprobs,
            "finish_reason": self.context.response.finish_reason.as_str(),
        });
        serde_json::to_value(ChatCompletionWire {
            id: self.response_id.as_deref().unwrap_or_default(),
            object: "chat.completion",
            created: self.context.created,
            model: self.context.model.as_str(),
            choices: vec![choice],
            usage,
            extra: filtered_extra(
                &self.context.response.extra,
                &["id", "object", "created", "model", "choices", "usage"],
            ),
        })
        .expect("serializing a Chat Completion cannot fail")
    }
}

fn usage_value(usage: &Usage) -> Value {
    let mut value = Map::from_iter([
        ("prompt_tokens".to_owned(), json!(usage.input_tokens)),
        ("completion_tokens".to_owned(), json!(usage.output_tokens)),
        (
            "total_tokens".to_owned(),
            json!(usage.input_tokens.saturating_add(usage.output_tokens)),
        ),
        (
            "prompt_tokens_details".to_owned(),
            json!({"cached_tokens": usage.cached_input_tokens.unwrap_or(0)}),
        ),
        (
            "completion_tokens_details".to_owned(),
            json!({"reasoning_tokens": usage.reasoning_tokens.unwrap_or(0)}),
        ),
    ]);
    for (key, extension) in &usage.extensions {
        value
            .entry(key.clone())
            .or_insert_with(|| extension.clone());
    }
    Value::Object(value)
}

fn error_envelope(error: &GatewayError) -> Value {
    serde_json::to_value(ChatErrorEnvelope {
        error: ChatErrorWire {
            kind: "gateway_error",
            code: error.code(),
            message: error.public_message(),
        },
    })
    .expect("serializing a Chat Completions error cannot fail")
}

fn filtered_extra(extra: &BTreeMap<String, Value>, reserved: &[&str]) -> BTreeMap<String, Value> {
    extra
        .iter()
        .filter(|(key, _)| !reserved.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}
