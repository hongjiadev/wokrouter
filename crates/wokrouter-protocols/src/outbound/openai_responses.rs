use bytes::Bytes;
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::{
    canonical::{CanonicalEvent, GatewayError, Usage},
    stream::encode_sse,
};

#[derive(Default)]
pub struct ResponsesCodec {
    terminal: bool,
    sequence_number: u64,
    response_id: Option<String>,
    output: Vec<ResponsesOutput>,
    usage: Option<Value>,
}

enum ResponsesOutput {
    Text {
        item_id: String,
        text: String,
    },
    Reasoning {
        item_id: String,
        text: String,
    },
    Tool {
        item_id: String,
        call_id: String,
        arguments: String,
    },
}

#[derive(Serialize)]
struct ResponsesEvent {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(flatten)]
    fields: Map<String, Value>,
}

impl ResponsesCodec {
    pub fn encode_response(events: &[CanonicalEvent]) -> Result<Value, GatewayError> {
        let mut codec = Self::default();
        let mut completed = None;

        for event in events {
            let (_, encoded) = codec.encode_event_value(event)?;
            match event {
                CanonicalEvent::Completed => {
                    completed = encoded.get("response").cloned();
                }
                CanonicalEvent::Failed(error) => {
                    completed = Some(json!({
                        "error": {
                            "type": "gateway_error",
                            "code": error.code(),
                            "message": error.public_message(),
                        }
                    }));
                }
                _ => {}
            }
        }

        if !codec.terminal {
            return Err(GatewayError::invalid_request());
        }
        completed.ok_or_else(GatewayError::invalid_request)
    }

    pub fn encode_event(&mut self, event: &CanonicalEvent) -> Result<Bytes, GatewayError> {
        let (event_name, value) = self.encode_event_value(event)?;
        Ok(encode_sse(Some(event_name), &value))
    }

    fn encode_event_value(
        &mut self,
        event: &CanonicalEvent,
    ) -> Result<(&'static str, Value), GatewayError> {
        if self.terminal {
            return Err(GatewayError::invalid_request());
        }

        let sequence_number = self.sequence_number;
        let (event_name, wire) = match event {
            CanonicalEvent::Created { response_id } => {
                if self.response_id.is_some() || response_id.is_empty() {
                    return Err(GatewayError::invalid_request());
                }
                self.response_id = Some(response_id.clone());
                (
                    "response.created",
                    wire_event(
                        "response.created",
                        [
                            (
                                "response",
                                json!({
                                    "id": response_id,
                                    "object": "response",
                                    "status": "in_progress",
                                }),
                            ),
                            ("sequence_number", json!(sequence_number)),
                        ],
                    ),
                )
            }
            CanonicalEvent::OutputTextDelta { item_id, delta } => {
                self.require_created()?;
                self.append_text(item_id, delta);
                (
                    "response.output_text.delta",
                    wire_event(
                        "response.output_text.delta",
                        [
                            ("item_id", json!(item_id)),
                            ("output_index", json!(0)),
                            ("content_index", json!(0)),
                            ("delta", json!(delta)),
                            ("sequence_number", json!(sequence_number)),
                        ],
                    ),
                )
            }
            CanonicalEvent::ReasoningDelta { item_id, delta } => {
                self.require_created()?;
                self.append_reasoning(item_id, delta);
                (
                    "response.reasoning_text.delta",
                    wire_event(
                        "response.reasoning_text.delta",
                        [
                            ("item_id", json!(item_id)),
                            ("output_index", json!(0)),
                            ("content_index", json!(0)),
                            ("delta", json!(delta)),
                            ("sequence_number", json!(sequence_number)),
                        ],
                    ),
                )
            }
            CanonicalEvent::ToolCallDelta {
                item_id,
                call_id,
                delta,
            } => {
                self.require_created()?;
                self.append_tool(item_id, call_id, delta)?;
                (
                    "response.function_call_arguments.delta",
                    wire_event(
                        "response.function_call_arguments.delta",
                        [
                            ("item_id", json!(item_id)),
                            ("call_id", json!(call_id)),
                            ("output_index", json!(0)),
                            ("delta", json!(delta)),
                            ("sequence_number", json!(sequence_number)),
                        ],
                    ),
                )
            }
            CanonicalEvent::Usage(usage) => {
                self.require_created()?;
                if self.usage.is_some() {
                    return Err(GatewayError::invalid_request());
                }
                let usage = usage_value(usage);
                self.usage = Some(usage.clone());
                (
                    "response.usage",
                    wire_event(
                        "response.usage",
                        [
                            ("usage", usage),
                            ("sequence_number", json!(sequence_number)),
                        ],
                    ),
                )
            }
            CanonicalEvent::Completed => {
                self.require_created()?;
                self.terminal = true;
                (
                    "response.completed",
                    wire_event(
                        "response.completed",
                        [
                            ("response", self.completed_response()),
                            ("sequence_number", json!(sequence_number)),
                        ],
                    ),
                )
            }
            CanonicalEvent::Failed(error) => {
                self.terminal = true;
                (
                    "error",
                    wire_event(
                        "error",
                        [
                            ("code", json!(error.code())),
                            ("message", json!(error.public_message())),
                            ("param", Value::Null),
                            ("sequence_number", json!(sequence_number)),
                        ],
                    ),
                )
            }
        };
        self.sequence_number += 1;
        Ok((event_name, wire))
    }

    fn require_created(&self) -> Result<(), GatewayError> {
        if self.response_id.is_some() {
            Ok(())
        } else {
            Err(GatewayError::invalid_request())
        }
    }

    fn append_text(&mut self, item_id: &str, delta: &str) {
        if let Some(ResponsesOutput::Text { text, .. }) = self
            .output
            .iter_mut()
            .find(|output| output.item_id() == item_id)
        {
            text.push_str(delta);
            return;
        }
        self.output.push(ResponsesOutput::Text {
            item_id: item_id.to_owned(),
            text: delta.to_owned(),
        });
    }

    fn append_reasoning(&mut self, item_id: &str, delta: &str) {
        if let Some(ResponsesOutput::Reasoning { text, .. }) = self
            .output
            .iter_mut()
            .find(|output| output.item_id() == item_id)
        {
            text.push_str(delta);
            return;
        }
        self.output.push(ResponsesOutput::Reasoning {
            item_id: item_id.to_owned(),
            text: delta.to_owned(),
        });
    }

    fn append_tool(
        &mut self,
        item_id: &str,
        call_id: &str,
        delta: &str,
    ) -> Result<(), GatewayError> {
        if let Some(ResponsesOutput::Tool {
            call_id: existing_call_id,
            arguments,
            ..
        }) = self
            .output
            .iter_mut()
            .find(|output| output.item_id() == item_id)
        {
            if existing_call_id != call_id {
                return Err(GatewayError::invalid_request());
            }
            arguments.push_str(delta);
            return Ok(());
        }
        self.output.push(ResponsesOutput::Tool {
            item_id: item_id.to_owned(),
            call_id: call_id.to_owned(),
            arguments: delta.to_owned(),
        });
        Ok(())
    }

    fn completed_response(&self) -> Value {
        let mut response = Map::from_iter([
            (
                "id".to_owned(),
                json!(self.response_id.as_deref().unwrap_or_default()),
            ),
            ("object".to_owned(), json!("response")),
            ("status".to_owned(), json!("completed")),
            (
                "output".to_owned(),
                Value::Array(self.output.iter().map(ResponsesOutput::value).collect()),
            ),
        ]);
        if let Some(usage) = &self.usage {
            response.insert("usage".to_owned(), usage.clone());
        }
        Value::Object(response)
    }
}

impl ResponsesOutput {
    fn item_id(&self) -> &str {
        match self {
            Self::Text { item_id, .. }
            | Self::Reasoning { item_id, .. }
            | Self::Tool { item_id, .. } => item_id,
        }
    }

    fn value(&self) -> Value {
        match self {
            Self::Text { item_id, text } => json!({
                "id": item_id,
                "type": "message",
                "role": "assistant",
                "status": "completed",
                "content": [{
                    "type": "output_text",
                    "text": text,
                    "annotations": [],
                }],
            }),
            Self::Reasoning { item_id, text } => json!({
                "id": item_id,
                "type": "reasoning",
                "summary": [{
                    "type": "summary_text",
                    "text": text,
                }],
            }),
            Self::Tool {
                item_id,
                call_id,
                arguments,
            } => json!({
                "id": item_id,
                "type": "function_call",
                "call_id": call_id,
                "arguments": arguments,
                "status": "completed",
            }),
        }
    }
}

fn wire_event<const N: usize>(kind: &'static str, fields: [(&str, Value); N]) -> Value {
    serde_json::to_value(ResponsesEvent {
        kind,
        fields: fields
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    })
    .expect("serializing a Responses event cannot fail")
}

fn usage_value(usage: &Usage) -> Value {
    let mut value = Map::from_iter([
        ("input_tokens".to_owned(), json!(usage.input_tokens)),
        (
            "input_tokens_details".to_owned(),
            json!({"cached_tokens": usage.cached_input_tokens.unwrap_or(0)}),
        ),
        ("output_tokens".to_owned(), json!(usage.output_tokens)),
        (
            "output_tokens_details".to_owned(),
            json!({"reasoning_tokens": usage.reasoning_tokens.unwrap_or(0)}),
        ),
        (
            "total_tokens".to_owned(),
            json!(usage.input_tokens.saturating_add(usage.output_tokens)),
        ),
    ]);
    for (key, extension) in &usage.extensions {
        value
            .entry(key.clone())
            .or_insert_with(|| extension.clone());
    }
    Value::Object(value)
}
