use std::{collections::HashMap, sync::Arc};

use wokrouter_protocols::canonical::{AdapterKind, CanonicalRequest, GatewayError};

use super::{CanonicalStream, ExecutionContext, UpstreamExecutor};

#[derive(Default)]
pub struct AdapterRegistry {
    executors: HashMap<AdapterKind, Arc<dyn UpstreamExecutor>>,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, kind: AdapterKind, executor: Arc<dyn UpstreamExecutor>) {
        self.executors.insert(kind, executor);
    }

    pub async fn execute(
        &self,
        kind: AdapterKind,
        context: ExecutionContext,
        request: CanonicalRequest,
    ) -> Result<CanonicalStream, GatewayError> {
        match kind {
            AdapterKind::OpenAiResponses
            | AdapterKind::OpenAiChat
            | AdapterKind::Anthropic
            | AdapterKind::Gemini
            | AdapterKind::AzureOpenAi
            | AdapterKind::Cursor => {
                self.executors
                    .get(&kind)
                    .ok_or_else(GatewayError::no_executor)?
                    .execute(context, request)
                    .await
            }
        }
    }
}
