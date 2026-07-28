use std::{collections::BTreeSet, fmt, time::Duration};

use reqwest::{Method, StatusCode};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    CoreConnection, ManagementError, WokCoreClient,
    http::ProtectedJsonOptions,
    management::{map_http_error, valid_identifier},
};

const CLIENT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CLIENT_RESPONSE_BYTES: usize = 64 * 1024;
const PROXY_SCOPE: &str = "proxy.use";
const TOKEN_PREFIX: &str = "wok_proxy_v1_";
const MAX_TOKEN_BYTES: usize = 4 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IntegrationRuntime {
    discovery: crate::discovery::ValidatedDiscovery,
    base_url: Url,
    installation_id: String,
    provider_protocols: BTreeSet<String>,
    capabilities: BTreeSet<String>,
}

impl IntegrationRuntime {
    pub fn base_url(&self) -> &str {
        self.base_url.as_str()
    }

    pub fn supports_protocol(&self, protocol: &str) -> bool {
        self.provider_protocols.contains(protocol)
    }

    pub fn supports_capability(&self, capability: &str) -> bool {
        self.capabilities.contains(capability)
    }

    pub fn instance_id(&self) -> String {
        self.discovery.instance_id.to_string()
    }

    pub fn installation_id(&self) -> &str {
        &self.installation_id
    }

    pub fn wokcore_version(&self) -> String {
        self.discovery.wokcore_version.to_string()
    }

    pub const fn management_api_major(&self) -> u32 {
        self.discovery.api_major
    }
}

pub struct IssuedProxyToken {
    client_id: String,
    token_id: String,
    token: SecretString,
    scopes: Vec<String>,
}

impl IssuedProxyToken {
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn token_id(&self) -> &str {
        &self.token_id
    }

    pub fn token(&self) -> &SecretString {
        &self.token
    }

    pub fn scopes(&self) -> &[String] {
        &self.scopes
    }
}

impl fmt::Debug for IssuedProxyToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IssuedProxyToken")
            .field("client_id", &self.client_id)
            .field("token_id", &self.token_id)
            .field("scopes", &self.scopes)
            .finish_non_exhaustive()
    }
}

impl WokCoreClient {
    pub async fn integration_runtime(&self) -> Result<IntegrationRuntime, ManagementError> {
        let handshake = match self.connection().await {
            CoreConnection::Missing => return Err(ManagementError::Missing),
            CoreConnection::Stopped => return Err(ManagementError::Stopped),
            CoreConnection::Incompatible(_) => return Err(ManagementError::Incompatible),
            CoreConnection::InvalidRuntime => return Err(ManagementError::InvalidRuntime),
            CoreConnection::Running(handshake) => handshake,
        };
        let discovery = self.management_discovery()?;
        if discovery.instance_id.to_string() != handshake.instance_id
            || discovery.wokcore_version.to_string() != handshake.version
            || discovery.api_major != handshake.management_api_major
        {
            return Err(ManagementError::InvalidRuntime);
        }
        let installation_id = handshake
            .installation_id
            .ok_or(ManagementError::Incompatible)?;
        let base_url = discovery
            .base_url
            .join("v1/")
            .map_err(|_| ManagementError::InvalidRuntime)?;
        Ok(IntegrationRuntime {
            discovery,
            base_url,
            installation_id,
            provider_protocols: handshake.provider_protocols,
            capabilities: handshake.capabilities,
        })
    }

    pub async fn issue_proxy_token(
        &self,
        management_token: &SecretString,
        client_id: &str,
    ) -> Result<IssuedProxyToken, ManagementError> {
        if !valid_identifier(client_id) {
            return Err(ManagementError::InvalidInput);
        }
        let discovery = self.management_discovery()?;
        self.issue_proxy_token_with_discovery(management_token, client_id, None, &discovery)
            .await
    }

    pub async fn issue_proxy_token_for_runtime(
        &self,
        runtime: &IntegrationRuntime,
        management_token: &SecretString,
        client_id: &str,
    ) -> Result<IssuedProxyToken, ManagementError> {
        self.issue_proxy_token_for_runtime_with_id(runtime, management_token, client_id, None)
            .await
    }

    pub async fn issue_proxy_token_for_runtime_with_preallocated_id(
        &self,
        runtime: &IntegrationRuntime,
        management_token: &SecretString,
        client_id: &str,
        token_id: &str,
    ) -> Result<IssuedProxyToken, ManagementError> {
        if !valid_uuid(token_id) {
            return Err(ManagementError::InvalidInput);
        }
        self.issue_proxy_token_for_runtime_with_id(
            runtime,
            management_token,
            client_id,
            Some(token_id),
        )
        .await
    }

    async fn issue_proxy_token_for_runtime_with_id(
        &self,
        runtime: &IntegrationRuntime,
        management_token: &SecretString,
        client_id: &str,
        token_id: Option<&str>,
    ) -> Result<IssuedProxyToken, ManagementError> {
        if !valid_identifier(client_id) {
            return Err(ManagementError::InvalidInput);
        }
        let issued = self
            .issue_proxy_token_with_discovery(
                management_token,
                client_id,
                token_id,
                &runtime.discovery,
            )
            .await?;
        if self.integration_runtime().await.as_ref() != Ok(runtime) {
            let _ = self
                .revoke_proxy_token_with_discovery(
                    management_token,
                    client_id,
                    issued.token_id(),
                    &runtime.discovery,
                )
                .await;
            return Err(ManagementError::InvalidRuntime);
        }
        Ok(issued)
    }

    async fn issue_proxy_token_with_discovery(
        &self,
        management_token: &SecretString,
        client_id: &str,
        token_id: Option<&str>,
        discovery: &crate::discovery::ValidatedDiscovery,
    ) -> Result<IssuedProxyToken, ManagementError> {
        let request = AuthorizeRequest {
            client_id,
            token_id,
            scopes: [PROXY_SCOPE],
        };
        let response: AuthorizeResponse = self
            .http
            .protected_json_body(
                discovery,
                Method::POST,
                "/wokcore/v1/clients/authorize",
                management_token,
                client_options(StatusCode::CREATED),
                &request,
            )
            .await
            .map_err(map_http_error)?;
        if response.client_id != client_id
            || !valid_uuid(&response.token_id)
            || token_id.is_some_and(|expected| response.token_id != expected)
            || response.scopes != [PROXY_SCOPE]
            || response.token.len() > MAX_TOKEN_BYTES
            || !response.token.starts_with(TOKEN_PREFIX)
            || response.token.len() == TOKEN_PREFIX.len()
            || response
                .token
                .bytes()
                .any(|byte| byte.is_ascii_whitespace())
        {
            return Err(ManagementError::InvalidResponse);
        }
        Ok(IssuedProxyToken {
            client_id: response.client_id,
            token_id: response.token_id,
            token: SecretString::from(response.token),
            scopes: response.scopes,
        })
    }

    pub async fn revoke_proxy_token(
        &self,
        management_token: &SecretString,
        client_id: &str,
        token_id: &str,
    ) -> Result<bool, ManagementError> {
        if !valid_identifier(client_id) || !valid_identifier(token_id) {
            return Err(ManagementError::InvalidInput);
        }
        let discovery = self.management_discovery()?;
        self.revoke_proxy_token_with_discovery(management_token, client_id, token_id, &discovery)
            .await
    }

    pub async fn revoke_proxy_token_for_runtime(
        &self,
        runtime: &IntegrationRuntime,
        management_token: &SecretString,
        client_id: &str,
        token_id: &str,
    ) -> Result<bool, ManagementError> {
        if !valid_identifier(client_id) || !valid_uuid(token_id) {
            return Err(ManagementError::InvalidInput);
        }
        if self.integration_runtime().await.as_ref() != Ok(runtime) {
            return Err(ManagementError::InvalidRuntime);
        }
        self.revoke_proxy_token_with_discovery(
            management_token,
            client_id,
            token_id,
            &runtime.discovery,
        )
        .await
    }

    pub async fn client_token_active_for_runtime(
        &self,
        runtime: &IntegrationRuntime,
        management_token: &SecretString,
        client_id: &str,
        token_id: &str,
    ) -> Result<bool, ManagementError> {
        if !valid_identifier(client_id) || !valid_uuid(token_id) {
            return Err(ManagementError::InvalidInput);
        }
        if self.integration_runtime().await.as_ref() != Ok(runtime) {
            return Err(ManagementError::InvalidRuntime);
        }
        let response: TokenStatusResponse = self
            .http
            .protected_json_no_body(
                &runtime.discovery,
                Method::GET,
                &format!("/wokcore/v1/clients/{client_id}/tokens/{token_id}"),
                management_token,
                client_options(StatusCode::OK),
            )
            .await
            .map_err(map_http_error)?;
        if self.integration_runtime().await.as_ref() != Ok(runtime) {
            return Err(ManagementError::InvalidRuntime);
        }
        Ok(response.active)
    }

    async fn revoke_proxy_token_with_discovery(
        &self,
        management_token: &SecretString,
        client_id: &str,
        token_id: &str,
        discovery: &crate::discovery::ValidatedDiscovery,
    ) -> Result<bool, ManagementError> {
        let response: RevokeResponse = self
            .http
            .protected_json_no_body(
                discovery,
                Method::DELETE,
                &format!("/wokcore/v1/clients/{client_id}/tokens/{token_id}"),
                management_token,
                client_options(StatusCode::OK),
            )
            .await
            .map_err(map_http_error)?;
        Ok(response.revoked)
    }
}

fn client_options(expected_status: StatusCode) -> ProtectedJsonOptions {
    ProtectedJsonOptions {
        request_timeout: CLIENT_TIMEOUT,
        max_response_bytes: MAX_CLIENT_RESPONSE_BYTES,
        expected_status,
    }
}

fn valid_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|parsed| parsed.to_string() == value)
}

#[derive(Serialize)]
struct AuthorizeRequest<'a> {
    client_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_id: Option<&'a str>,
    scopes: [&'static str; 1],
}

#[derive(Deserialize)]
struct AuthorizeResponse {
    client_id: String,
    token_id: String,
    token: String,
    scopes: Vec<String>,
}

#[derive(Deserialize)]
struct RevokeResponse {
    revoked: bool,
}

#[derive(Deserialize)]
struct TokenStatusResponse {
    active: bool,
}
