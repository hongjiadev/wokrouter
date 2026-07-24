#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientProtocol {
    OpenAiResponses,
    OpenAiChatCompletions,
    AnthropicMessages,
    AnthropicCountTokens,
    OpenAiModels,
    OpenAiImageGenerations,
    OpenAiImageEdits,
}

impl ClientProtocol {
    pub(crate) fn expects_json_body(self) -> bool {
        !matches!(self, Self::OpenAiModels)
    }

    pub(crate) fn is_anthropic(self) -> bool {
        matches!(self, Self::AnthropicMessages | Self::AnthropicCountTokens)
    }
}

pub struct ProtocolRegistry;

impl ProtocolRegistry {
    pub fn resolve(path: &str) -> Option<ClientProtocol> {
        match path {
            "/v1/responses" => Some(ClientProtocol::OpenAiResponses),
            "/v1/chat/completions" => Some(ClientProtocol::OpenAiChatCompletions),
            "/v1/messages" => Some(ClientProtocol::AnthropicMessages),
            "/v1/messages/count_tokens" => Some(ClientProtocol::AnthropicCountTokens),
            "/v1/models" => Some(ClientProtocol::OpenAiModels),
            "/v1/images/generations" => Some(ClientProtocol::OpenAiImageGenerations),
            "/v1/images/edits" => Some(ClientProtocol::OpenAiImageEdits),
            _ => None,
        }
    }
}
