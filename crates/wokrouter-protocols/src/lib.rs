pub mod canonical;
pub mod stream;

mod inbound;
mod outbound;

pub use outbound::{
    ChatCodec, ChatEncodeContext, ChatFinishReason, ChatResponseTemplate, ResponsesCodec,
    ResponsesEncodeContext, ResponsesResponseTemplate,
};

pub(crate) fn valid_chat_function_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}
