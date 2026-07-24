use std::collections::BTreeMap;

use axum::Json;
use serde::Serialize;
use wokrouter_protocols::canonical::GatewayError;

const MAX_MODELS: usize = 4096;
const MAX_MODEL_ID_BYTES: usize = 512;
const MAX_OWNER_BYTES: usize = 128;
const MAX_CAPABILITIES: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PublicModelMetadata {
    id: String,
    object: &'static str,
    created: u64,
    owned_by: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    capabilities: BTreeMap<String, bool>,
}

impl PublicModelMetadata {
    pub fn new(id: impl Into<String>, owned_by: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            object: "model",
            created: 0,
            owned_by: owned_by.into(),
            capabilities: BTreeMap::new(),
        }
    }

    pub fn with_capability(mut self, name: impl Into<String>, supported: bool) -> Self {
        self.capabilities.insert(name.into(), supported);
        self
    }

    fn validate(&self) -> Result<(), GatewayError> {
        validate_public_text(&self.id, MAX_MODEL_ID_BYTES)?;
        validate_public_text(&self.owned_by, MAX_OWNER_BYTES)?;
        if self.capabilities.len() > MAX_CAPABILITIES {
            return Err(GatewayError::invalid_request());
        }
        for name in self.capabilities.keys() {
            if name.is_empty()
                || name.len() > 64
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            {
                return Err(GatewayError::invalid_request());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModelCatalogSnapshot {
    models: Vec<PublicModelMetadata>,
}

impl ModelCatalogSnapshot {
    pub fn new(mut models: Vec<PublicModelMetadata>) -> Result<Self, GatewayError> {
        if models.len() > MAX_MODELS {
            return Err(GatewayError::invalid_request());
        }
        for model in &models {
            model.validate()?;
        }
        models.sort_by(|left, right| left.id.cmp(&right.id));
        if models.windows(2).any(|models| models[0].id == models[1].id) {
            return Err(GatewayError::invalid_request());
        }
        Ok(Self { models })
    }

    pub fn models(&self) -> &[PublicModelMetadata] {
        &self.models
    }
}

#[derive(Serialize)]
pub(crate) struct OpenAiModelsResponse {
    object: &'static str,
    data: Vec<PublicModelMetadata>,
}

pub(crate) async fn models(
    axum::extract::State(state): axum::extract::State<super::DataPlaneState>,
) -> Json<OpenAiModelsResponse> {
    let catalog = state.snapshot().model_catalog();
    Json(OpenAiModelsResponse {
        object: "list",
        data: catalog.models,
    })
}

fn validate_public_text(value: &str, limit: usize) -> Result<(), GatewayError> {
    if value.is_empty()
        || value.len() > limit
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(GatewayError::invalid_request());
    }
    Ok(())
}
