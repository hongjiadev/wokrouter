use std::{collections::BTreeMap, time::Duration};

use reqwest::{Method, StatusCode};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::{
    ManagementError, WokCoreClient,
    http::ProtectedJsonOptions,
    management::{map_http_error, valid_identifier, valid_secret_ref},
};

const PROVIDER_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_PROVIDER_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

fn provider_json_options(expected_status: StatusCode) -> ProtectedJsonOptions {
    ProtectedJsonOptions {
        request_timeout: PROVIDER_TIMEOUT,
        max_response_bytes: MAX_PROVIDER_RESPONSE_BYTES,
        expected_status,
    }
}

fn secret_json_options(expected_status: StatusCode) -> ProtectedJsonOptions {
    ProtectedJsonOptions {
        request_timeout: PROVIDER_TIMEOUT,
        max_response_bytes: 64 * 1024,
        expected_status,
    }
}

impl WokCoreClient {
    pub async fn provider_catalog(
        &self,
        token: &SecretString,
    ) -> Result<ProviderCatalogResponse, ManagementError> {
        let discovery = self.management_discovery()?;
        let response = self
            .http
            .protected_json_query(
                &discovery,
                "/wokcore/v1/providers/catalog",
                &[],
                token,
                PROVIDER_TIMEOUT,
                MAX_PROVIDER_RESPONSE_BYTES,
            )
            .await
            .map_err(map_http_error)?;
        validate_catalog(response)
    }

    pub async fn provider_runtime(
        &self,
        token: &SecretString,
    ) -> Result<ProviderRuntimeResponse, ManagementError> {
        let discovery = self.management_discovery()?;
        let response = self
            .http
            .protected_json_query(
                &discovery,
                "/wokcore/v1/providers/runtime",
                &[],
                token,
                PROVIDER_TIMEOUT,
                MAX_PROVIDER_RESPONSE_BYTES,
            )
            .await
            .map_err(map_http_error)?;
        validate_runtime(response)
    }

    pub async fn provider_models(
        &self,
        token: &SecretString,
    ) -> Result<ProviderModelsResponse, ManagementError> {
        let discovery = self.management_discovery()?;
        let response = self
            .http
            .protected_json_query(
                &discovery,
                "/wokcore/v1/providers/models",
                &[],
                token,
                PROVIDER_TIMEOUT,
                MAX_PROVIDER_RESPONSE_BYTES,
            )
            .await
            .map_err(map_http_error)?;
        validate_models(response)
    }

    pub async fn validate_provider_config(
        &self,
        token: &SecretString,
        candidate: &ProviderCandidate,
    ) -> Result<ProviderValidationResponse, ManagementError> {
        validate_candidate(candidate)?;
        let discovery = self.management_discovery()?;
        let response = self
            .http
            .protected_json_body(
                &discovery,
                Method::POST,
                "/wokcore/v1/providers/config/validate",
                token,
                provider_json_options(StatusCode::OK),
                candidate,
            )
            .await
            .map_err(map_http_error)?;
        validate_validation_response(response)
    }

    pub async fn commit_provider_config(
        &self,
        token: &SecretString,
        request: &ProviderCommitRequest,
    ) -> Result<ProviderCommitResponse, ManagementError> {
        validate_candidate(&ProviderCandidate {
            providers: request.providers.clone(),
            routing: request.routing.clone(),
        })?;
        let discovery = self.management_discovery()?;
        let response = self
            .http
            .protected_json_body(
                &discovery,
                Method::PUT,
                "/wokcore/v1/providers/config",
                token,
                provider_json_options(StatusCode::OK),
                request,
            )
            .await
            .map_err(map_http_error)?;
        validate_commit_response(response)
    }

    pub async fn reload_providers(
        &self,
        token: &SecretString,
    ) -> Result<ProviderCommitResponse, ManagementError> {
        let discovery = self.management_discovery()?;
        let response = self
            .http
            .protected_json_no_body(
                &discovery,
                Method::POST,
                "/wokcore/v1/providers/reload",
                token,
                provider_json_options(StatusCode::OK),
            )
            .await
            .map_err(map_http_error)?;
        validate_commit_response(response)
    }

    pub async fn create_provider_secret(
        &self,
        token: &SecretString,
        request: &ProviderSecretCreate,
    ) -> Result<ProviderSecretResponse, ManagementError> {
        request.validate()?;
        let discovery = self.management_discovery()?;
        let body = request.serialized()?;
        let response = self
            .http
            .protected_secret_json(
                &discovery,
                Method::POST,
                "/wokcore/v1/provider-secrets",
                token,
                secret_json_options(StatusCode::CREATED),
                body,
            )
            .await
            .map_err(map_http_error)?;
        validate_secret_response(response)
    }

    pub async fn replace_provider_secret(
        &self,
        token: &SecretString,
        secret_ref: &str,
        secret: &SecretString,
    ) -> Result<ProviderSecretResponse, ManagementError> {
        if !valid_secret_ref(secret_ref) || secret.expose_secret().is_empty() {
            return Err(ManagementError::InvalidInput);
        }
        let body = serialize_replacement(secret)?;
        let discovery = self.management_discovery()?;
        let response = self
            .http
            .protected_secret_json(
                &discovery,
                Method::PUT,
                &format!("/wokcore/v1/provider-secrets/{secret_ref}"),
                token,
                secret_json_options(StatusCode::OK),
                body,
            )
            .await
            .map_err(map_http_error)?;
        validate_secret_response(response)
    }

    pub async fn delete_provider_secret(
        &self,
        token: &SecretString,
        secret_ref: &str,
    ) -> Result<ProviderSecretResponse, ManagementError> {
        if !valid_secret_ref(secret_ref) {
            return Err(ManagementError::InvalidInput);
        }
        let discovery = self.management_discovery()?;
        let response = self
            .http
            .protected_json_no_body(
                &discovery,
                Method::DELETE,
                &format!("/wokcore/v1/provider-secrets/{secret_ref}"),
                token,
                secret_json_options(StatusCode::OK),
            )
            .await
            .map_err(map_http_error)?;
        validate_secret_response(response)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderCatalogResponse {
    pub schema_version: u32,
    pub catalog_schema_version: u32,
    pub baseline_commit: String,
    pub providers: Vec<ProviderDefinition>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderDefinition {
    pub id: String,
    pub label: String,
    pub adapter: ProviderAdapter,
    pub base_url: String,
    pub auth_kind: ProviderAuthKind,
    pub endpoint_policy: EndpointPolicy,
    pub model_source: ModelSource,
    pub aliases: Vec<String>,
    pub models: Vec<String>,
    pub default_model: Option<String>,
    pub allow_endpoint_override: bool,
    pub key_optional: bool,
    pub allow_key_auth_override: bool,
    pub reasoning_efforts: Vec<String>,
    pub reasoning_effort_map: BTreeMap<String, String>,
    pub capabilities: ProviderCapabilities,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAdapter {
    OpenAiResponses,
    OpenAiChat,
    Anthropic,
    Google,
    AzureOpenAi,
    Cursor,
    Kiro,
    MimoFree,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAuthKind {
    Forward,
    Oauth,
    Key,
    Local,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointPolicy {
    PublicHttps,
    HttpsTemplate,
    LoopbackHttp,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSource {
    None,
    Static,
    Live,
    Hybrid,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderCapabilities {
    pub text: bool,
    pub streaming: bool,
    pub tools: bool,
    pub vision: bool,
    pub images: bool,
    pub reasoning: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderRuntimeResponse {
    pub schema_version: u32,
    pub revision: u64,
    pub snapshot_revision: u64,
    pub reload_status: ProviderReloadStatus,
    pub provider_count: u64,
    pub models: Vec<PublicModel>,
    pub providers: ProviderConfig,
    pub routing: RoutingConfig,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderReloadStatus {
    Ready,
    Failed,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderConfig {
    pub instances: Vec<ProviderInstance>,
    pub accounts: Vec<ProviderAccount>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderInstance {
    pub id: String,
    pub catalog_id: String,
    pub enabled: bool,
    pub endpoint: Option<String>,
    pub allow_private_network: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderAccount {
    pub id: String,
    pub provider: String,
    pub enabled: bool,
    pub auth: ProviderAccountAuth,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderAccountAuth {
    Forward {
        credential: String,
    },
    Oauth {
        access: String,
        refresh: Option<String>,
    },
    ApiKey {
        secret: String,
    },
    Local,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoutingConfig {
    pub aliases: Vec<ModelAlias>,
    pub rules: Vec<RouteRule>,
    pub default: Option<RouteTarget>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelAlias {
    pub alias: String,
    pub target: RouteTarget,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RouteRule {
    pub client_id: Option<String>,
    pub model: Option<String>,
    pub target: RouteTarget,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RouteTarget {
    pub provider: String,
    pub model: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PublicModel {
    pub id: String,
    pub owned_by: String,
    pub capabilities: ProviderCapabilities,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderModelsResponse {
    pub schema_version: u32,
    pub models: Vec<PublicModel>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderCandidate {
    pub providers: ProviderConfig,
    pub routing: RoutingConfig,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderValidationResponse {
    pub schema_version: u32,
    pub valid: bool,
    pub provider_count: u64,
    pub models: Vec<PublicModel>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderCommitRequest {
    pub expected_revision: u64,
    pub providers: ProviderConfig,
    pub routing: RoutingConfig,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderCommitResponse {
    pub schema_version: u32,
    pub revision: u64,
    pub snapshot_revision: u64,
    pub provider_count: u64,
    pub models: Vec<PublicModel>,
}

pub struct ProviderSecretCreate {
    provider_id: String,
    account_id: Option<String>,
    purpose: ProviderSecretPurpose,
    secret: SecretString,
}

impl ProviderSecretCreate {
    pub fn new(
        provider_id: impl Into<String>,
        account_id: Option<String>,
        purpose: ProviderSecretPurpose,
        secret: SecretString,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            account_id,
            purpose,
            secret,
        }
    }

    fn validate(&self) -> Result<(), ManagementError> {
        if !valid_identifier(&self.provider_id)
            || self
                .account_id
                .as_ref()
                .is_some_and(|account_id| !valid_identifier(account_id))
            || self.secret.expose_secret().is_empty()
        {
            return Err(ManagementError::InvalidInput);
        }
        Ok(())
    }

    fn serialized(&self) -> Result<Zeroizing<Vec<u8>>, ManagementError> {
        #[derive(Serialize)]
        struct Request<'a> {
            provider_id: &'a str,
            account_id: Option<&'a str>,
            purpose: ProviderSecretPurpose,
            secret: &'a str,
        }

        serde_json::to_vec(&Request {
            provider_id: &self.provider_id,
            account_id: self.account_id.as_deref(),
            purpose: self.purpose,
            secret: self.secret.expose_secret(),
        })
        .map(Zeroizing::new)
        .map_err(|_| ManagementError::InvalidInput)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSecretPurpose {
    ApiKey,
    OauthAccess,
    OauthRefresh,
    LanToken,
    Auxiliary,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderSecretResponse {
    pub schema_version: u32,
    pub operation: ProviderSecretOperation,
    pub secret_ref: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSecretOperation {
    Created,
    Replaced,
    Deleted,
}

fn serialize_replacement(secret: &SecretString) -> Result<Zeroizing<Vec<u8>>, ManagementError> {
    #[derive(Serialize)]
    struct Request<'a> {
        secret: &'a str,
    }

    serde_json::to_vec(&Request {
        secret: secret.expose_secret(),
    })
    .map(Zeroizing::new)
    .map_err(|_| ManagementError::InvalidInput)
}

fn validate_catalog(
    response: ProviderCatalogResponse,
) -> Result<ProviderCatalogResponse, ManagementError> {
    let valid = response.schema_version == 1
        && response.catalog_schema_version == 1
        && !response.baseline_commit.is_empty()
        && response.baseline_commit.len() <= 64
        && (1..=256).contains(&response.providers.len())
        && response.providers.iter().all(valid_definition);
    if valid {
        Ok(response)
    } else {
        Err(ManagementError::InvalidResponse)
    }
}

fn valid_definition(provider: &ProviderDefinition) -> bool {
    valid_identifier(&provider.id)
        && !provider.label.is_empty()
        && provider.label.len() <= 256
        && !provider.base_url.is_empty()
        && provider.base_url.len() <= 2048
        && provider.aliases.len() <= 32
        && provider.aliases.iter().all(|alias| valid_identifier(alias))
        && provider.models.len() <= 512
        && provider
            .models
            .iter()
            .all(|model| !model.is_empty() && model.len() <= 256)
        && provider
            .default_model
            .as_ref()
            .is_none_or(|model| model.len() <= 256)
        && provider.reasoning_efforts.len() <= 16
        && provider.reasoning_effort_map.len() <= 32
}

fn validate_runtime(
    response: ProviderRuntimeResponse,
) -> Result<ProviderRuntimeResponse, ManagementError> {
    let valid = response.schema_version == 1
        && response.provider_count <= 64
        && valid_provider_config(&response.providers)
        && valid_routing(&response.routing)
        && response.models.iter().all(valid_public_model);
    if valid {
        Ok(response)
    } else {
        Err(ManagementError::InvalidResponse)
    }
}

fn validate_models(
    response: ProviderModelsResponse,
) -> Result<ProviderModelsResponse, ManagementError> {
    if response.schema_version == 1 && response.models.iter().all(valid_public_model) {
        Ok(response)
    } else {
        Err(ManagementError::InvalidResponse)
    }
}

fn validate_candidate(candidate: &ProviderCandidate) -> Result<(), ManagementError> {
    if valid_provider_config(&candidate.providers) && valid_routing(&candidate.routing) {
        Ok(())
    } else {
        Err(ManagementError::InvalidInput)
    }
}

fn valid_provider_config(config: &ProviderConfig) -> bool {
    config.instances.len() <= 64
        && config.accounts.len() <= 256
        && config.instances.iter().all(|instance| {
            valid_identifier(&instance.id)
                && valid_identifier(&instance.catalog_id)
                && instance
                    .endpoint
                    .as_ref()
                    .is_none_or(|endpoint| endpoint.len() <= 2048)
        })
        && config.accounts.iter().all(|account| {
            valid_identifier(&account.id)
                && valid_identifier(&account.provider)
                && match &account.auth {
                    ProviderAccountAuth::Forward { credential } => valid_secret_ref(credential),
                    ProviderAccountAuth::Oauth { access, refresh } => {
                        valid_secret_ref(access)
                            && refresh.as_ref().is_none_or(|value| valid_secret_ref(value))
                    }
                    ProviderAccountAuth::ApiKey { secret } => valid_secret_ref(secret),
                    ProviderAccountAuth::Local => true,
                }
        })
}

fn valid_routing(routing: &RoutingConfig) -> bool {
    routing.aliases.len() <= 1024
        && routing.rules.len() <= 1024
        && routing.aliases.iter().all(|alias| {
            !alias.alias.is_empty() && alias.alias.len() <= 256 && valid_route_target(&alias.target)
        })
        && routing.rules.iter().all(|rule| {
            rule.client_id
                .as_ref()
                .is_none_or(|client_id| valid_identifier(client_id))
                && rule.model.as_ref().is_none_or(|model| model.len() <= 256)
                && valid_route_target(&rule.target)
        })
        && routing.default.as_ref().is_none_or(valid_route_target)
}

fn valid_route_target(target: &RouteTarget) -> bool {
    valid_identifier(&target.provider) && !target.model.is_empty() && target.model.len() <= 256
}

fn valid_public_model(model: &PublicModel) -> bool {
    !model.id.is_empty() && model.id.len() <= 256 && valid_identifier(&model.owned_by)
}

fn validate_validation_response(
    response: ProviderValidationResponse,
) -> Result<ProviderValidationResponse, ManagementError> {
    if response.schema_version == 1
        && response.valid
        && response.provider_count <= 64
        && response.models.iter().all(valid_public_model)
    {
        Ok(response)
    } else {
        Err(ManagementError::InvalidResponse)
    }
}

fn validate_commit_response(
    response: ProviderCommitResponse,
) -> Result<ProviderCommitResponse, ManagementError> {
    if response.schema_version == 1
        && response.provider_count <= 64
        && response.models.iter().all(valid_public_model)
    {
        Ok(response)
    } else {
        Err(ManagementError::InvalidResponse)
    }
}

fn validate_secret_response(
    response: ProviderSecretResponse,
) -> Result<ProviderSecretResponse, ManagementError> {
    if response.schema_version == 1 && valid_secret_ref(&response.secret_ref) {
        Ok(response)
    } else {
        Err(ManagementError::InvalidResponse)
    }
}
