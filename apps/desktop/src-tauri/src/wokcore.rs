use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wokrouter_platform::{AppPaths, SelectedWokCoreRuntime};
use wokrouter_storage::{NativeWokCoreTokenVault, TokenVaultError, WokCoreTokenVault};
use wokrouter_wokcore_client::{
    DiagnosticExportQuery, DiagnosticLogQuery, DiagnosticLogs, ManagementError, ProviderCandidate,
    ProviderCatalogResponse, ProviderCommitRequest, ProviderCommitResponse, ProviderModelsResponse,
    ProviderRuntimeResponse, ProviderSecretCreate, ProviderSecretPurpose, ProviderSecretResponse,
    ProviderValidationResponse, SessionList, SessionMessageQuery, SessionMessages, SessionQuery,
    UsageQuery, UsageResponse, WokCoreClient,
};
use zeroize::Zeroizing;

use crate::runtime::DesktopRuntimeState;

pub(crate) struct ManagementState {
    runtime: Arc<DesktopRuntimeState>,
    management: Result<DesktopManagement, DesktopApiError>,
}

impl ManagementState {
    pub(crate) fn discover(runtime: Arc<DesktopRuntimeState>) -> Self {
        Self {
            runtime,
            management: DesktopManagement::discover(),
        }
    }

    fn get(&self) -> Result<&DesktopManagement, DesktopApiError> {
        self.management.as_ref().map_err(|error| *error)
    }

    async fn selected(&self) -> Result<&SelectedWokCoreRuntime, DesktopApiError> {
        self.runtime
            .selected()
            .await
            .map_err(|_| DesktopApiError::initialization())
    }

    pub(crate) async fn command(
        &self,
    ) -> Result<(&DesktopManagement, &SelectedWokCoreRuntime), DesktopApiError> {
        Ok((self.get()?, self.selected().await?))
    }
}

pub(crate) struct DesktopManagement {
    export_dir: PathBuf,
    #[cfg(test)]
    test_token: Option<SecretString>,
}

impl DesktopManagement {
    fn discover() -> Result<Self, DesktopApiError> {
        let paths = AppPaths::discover().map_err(|_| DesktopApiError::initialization())?;
        let export_dir = paths
            .log_dir
            .parent()
            .ok_or_else(DesktopApiError::initialization)?
            .join("diagnostic-exports");
        Ok(Self {
            export_dir,
            #[cfg(test)]
            test_token: None,
        })
    }

    async fn token(&self) -> Result<SecretString, DesktopApiError> {
        #[cfg(test)]
        if let Some(token) = &self.test_token {
            use secrecy::ExposeSecret;

            return Ok(SecretString::from(token.expose_secret().to_owned()));
        }

        NativeWokCoreTokenVault::new()
            .load()
            .await
            .map_err(map_vault_error)?
            .ok_or_else(DesktopApiError::authorization_required)
    }

    async fn provider_catalog(
        &self,
        client: &WokCoreClient,
    ) -> Result<ProviderCatalogResponse, DesktopApiError> {
        let token = self.token().await?;
        client
            .provider_catalog(&token)
            .await
            .map_err(map_management_error)
    }

    async fn provider_runtime(
        &self,
        client: &WokCoreClient,
    ) -> Result<ProviderRuntimeResponse, DesktopApiError> {
        let token = self.token().await?;
        client
            .provider_runtime(&token)
            .await
            .map_err(map_management_error)
    }

    async fn provider_models(
        &self,
        client: &WokCoreClient,
    ) -> Result<ProviderModelsResponse, DesktopApiError> {
        let token = self.token().await?;
        client
            .provider_models(&token)
            .await
            .map_err(map_management_error)
    }

    async fn validate_provider_config(
        &self,
        client: &WokCoreClient,
        candidate: &ProviderCandidate,
    ) -> Result<ProviderValidationResponse, DesktopApiError> {
        let token = self.token().await?;
        client
            .validate_provider_config(&token, candidate)
            .await
            .map_err(map_management_error)
    }

    async fn commit_provider_config(
        &self,
        client: &WokCoreClient,
        request: &ProviderCommitRequest,
    ) -> Result<ProviderCommitResponse, DesktopApiError> {
        let token = self.token().await?;
        client
            .commit_provider_config(&token, request)
            .await
            .map_err(map_management_error)
    }

    async fn reload_providers(
        &self,
        client: &WokCoreClient,
    ) -> Result<ProviderCommitResponse, DesktopApiError> {
        let token = self.token().await?;
        client
            .reload_providers(&token)
            .await
            .map_err(map_management_error)
    }

    async fn create_provider_secret(
        &self,
        client: &WokCoreClient,
        request: ProviderSecretCreateCommand,
    ) -> Result<ProviderSecretResponse, DesktopApiError> {
        let token = self.token().await?;
        let request = request.into_client_request();
        client
            .create_provider_secret(&token, &request)
            .await
            .map_err(map_management_error)
    }

    async fn replace_provider_secret(
        &self,
        client: &WokCoreClient,
        request: ProviderSecretReplaceCommand,
    ) -> Result<ProviderSecretResponse, DesktopApiError> {
        let token = self.token().await?;
        let mut secret = Zeroizing::new(request.secret);
        let secret = SecretString::from(std::mem::take(&mut *secret));
        client
            .replace_provider_secret(&token, &request.secret_ref, &secret)
            .await
            .map_err(map_management_error)
    }

    async fn delete_provider_secret(
        &self,
        client: &WokCoreClient,
        secret_ref: &str,
    ) -> Result<ProviderSecretResponse, DesktopApiError> {
        let token = self.token().await?;
        client
            .delete_provider_secret(&token, secret_ref)
            .await
            .map_err(map_management_error)
    }

    async fn list_sessions(
        &self,
        client: &WokCoreClient,
        query: &SessionQuery,
    ) -> Result<SessionList, DesktopApiError> {
        let token = self.token().await?;
        client
            .list_sessions(&token, query)
            .await
            .map_err(map_management_error)
    }

    async fn session_messages(
        &self,
        client: &WokCoreClient,
        session_key: &str,
        query: &SessionMessageQuery,
    ) -> Result<SessionMessages, DesktopApiError> {
        let token = self.token().await?;
        client
            .session_messages(&token, session_key, query)
            .await
            .map_err(map_management_error)
    }

    async fn usage(
        &self,
        client: &WokCoreClient,
        query: &UsageQuery,
    ) -> Result<UsageResponse, DesktopApiError> {
        let token = self.token().await?;
        client
            .usage(&token, query)
            .await
            .map_err(map_management_error)
    }

    async fn diagnostic_logs(
        &self,
        client: &WokCoreClient,
        query: &DiagnosticLogQuery,
    ) -> Result<DiagnosticLogs, DesktopApiError> {
        let token = self.token().await?;
        client
            .diagnostic_logs(&token, query)
            .await
            .map_err(map_management_error)
    }

    async fn export_diagnostics(
        &self,
        client: &WokCoreClient,
        query: &DiagnosticExportQuery,
    ) -> Result<DiagnosticExportReceipt, DesktopApiError> {
        let token = self.token().await?;
        let bytes = client
            .export_diagnostics(&token, query)
            .await
            .map_err(map_management_error)?;
        write_diagnostic_export(&self.export_dir, &bytes)
    }
}

#[cfg(test)]
impl ManagementState {
    pub(crate) fn for_test(
        runtime: Arc<DesktopRuntimeState>,
        export_dir: PathBuf,
        token: &str,
    ) -> Self {
        Self {
            runtime,
            management: Ok(DesktopManagement {
                export_dir,
                test_token: Some(SecretString::from(token.to_owned())),
            }),
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct ProviderSecretCreateCommand {
    provider_id: String,
    account_id: Option<String>,
    purpose: ProviderSecretPurpose,
    secret: String,
}

impl ProviderSecretCreateCommand {
    fn into_client_request(self) -> ProviderSecretCreate {
        let mut secret = Zeroizing::new(self.secret);
        ProviderSecretCreate::new(
            self.provider_id,
            self.account_id,
            self.purpose,
            SecretString::from(std::mem::take(&mut *secret)),
        )
    }
}

#[derive(Deserialize)]
pub(crate) struct ProviderSecretReplaceCommand {
    secret_ref: String,
    secret: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct DiagnosticExportReceipt {
    file_name: String,
    bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct DesktopApiError {
    code: &'static str,
}

impl DesktopApiError {
    const fn initialization() -> Self {
        Self {
            code: "initialization_failed",
        }
    }

    const fn authorization_required() -> Self {
        Self {
            code: "authorization_required",
        }
    }
}

fn map_management_error(error: ManagementError) -> DesktopApiError {
    let code = match error {
        ManagementError::Missing => "runtime_missing",
        ManagementError::Stopped => "runtime_stopped",
        ManagementError::Incompatible => "api_incompatible",
        ManagementError::InvalidRuntime => "runtime_invalid",
        ManagementError::Unauthorized => "authorization_required",
        ManagementError::Forbidden => "capability_denied",
        ManagementError::Conflict => "revision_conflict",
        ManagementError::InvalidInput => "invalid_request",
        ManagementError::InvalidResponse => "invalid_response",
    };
    DesktopApiError { code }
}

fn map_vault_error(error: TokenVaultError) -> DesktopApiError {
    let code = match error {
        TokenVaultError::Unavailable => "credential_service_unavailable",
        TokenVaultError::BackendFailure | TokenVaultError::InvalidEncoding => {
            "credential_service_failed"
        }
    };
    DesktopApiError { code }
}

fn write_diagnostic_export(
    export_dir: &Path,
    bytes: &[u8],
) -> Result<DiagnosticExportReceipt, DesktopApiError> {
    std::fs::create_dir_all(export_dir).map_err(|_| DesktopApiError {
        code: "export_failed",
    })?;
    let file_name = format!("wokcore-diagnostics-{}.zip", Uuid::new_v4().simple());
    let path = export_dir.join(&file_name);
    let mut file = open_private_export(&path)?;
    file.write_all(bytes).map_err(|_| DesktopApiError {
        code: "export_failed",
    })?;
    file.sync_all().map_err(|_| DesktopApiError {
        code: "export_failed",
    })?;
    Ok(DiagnosticExportReceipt {
        file_name,
        bytes: bytes.len(),
    })
}

#[cfg(unix)]
fn open_private_export(path: &Path) -> Result<File, DesktopApiError> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| DesktopApiError {
            code: "export_failed",
        })
}

#[cfg(not(unix))]
fn open_private_export(path: &Path) -> Result<File, DesktopApiError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| DesktopApiError {
            code: "export_failed",
        })
}

#[tauri::command]
pub(crate) async fn provider_catalog(
    state: tauri::State<'_, ManagementState>,
) -> Result<ProviderCatalogResponse, DesktopApiError> {
    provider_catalog_inner(&state).await
}

pub(crate) async fn provider_catalog_inner(
    state: &ManagementState,
) -> Result<ProviderCatalogResponse, DesktopApiError> {
    let (management, runtime) = state.command().await?;
    management.provider_catalog(runtime.client()).await
}

#[tauri::command]
pub(crate) async fn provider_runtime(
    state: tauri::State<'_, ManagementState>,
) -> Result<ProviderRuntimeResponse, DesktopApiError> {
    let (management, runtime) = state.command().await?;
    management.provider_runtime(runtime.client()).await
}

#[tauri::command]
pub(crate) async fn provider_models(
    state: tauri::State<'_, ManagementState>,
) -> Result<ProviderModelsResponse, DesktopApiError> {
    let (management, runtime) = state.command().await?;
    management.provider_models(runtime.client()).await
}

#[tauri::command]
pub(crate) async fn validate_provider_config(
    state: tauri::State<'_, ManagementState>,
    candidate: ProviderCandidate,
) -> Result<ProviderValidationResponse, DesktopApiError> {
    let (management, runtime) = state.command().await?;
    management
        .validate_provider_config(runtime.client(), &candidate)
        .await
}

#[tauri::command]
pub(crate) async fn commit_provider_config(
    state: tauri::State<'_, ManagementState>,
    request: ProviderCommitRequest,
) -> Result<ProviderCommitResponse, DesktopApiError> {
    let (management, runtime) = state.command().await?;
    management
        .commit_provider_config(runtime.client(), &request)
        .await
}

#[tauri::command]
pub(crate) async fn reload_providers(
    state: tauri::State<'_, ManagementState>,
) -> Result<ProviderCommitResponse, DesktopApiError> {
    let (management, runtime) = state.command().await?;
    management.reload_providers(runtime.client()).await
}

#[tauri::command]
pub(crate) async fn create_provider_secret(
    state: tauri::State<'_, ManagementState>,
    request: ProviderSecretCreateCommand,
) -> Result<ProviderSecretResponse, DesktopApiError> {
    let (management, runtime) = state.command().await?;
    management
        .create_provider_secret(runtime.client(), request)
        .await
}

#[tauri::command]
pub(crate) async fn replace_provider_secret(
    state: tauri::State<'_, ManagementState>,
    request: ProviderSecretReplaceCommand,
) -> Result<ProviderSecretResponse, DesktopApiError> {
    let (management, runtime) = state.command().await?;
    management
        .replace_provider_secret(runtime.client(), request)
        .await
}

#[tauri::command]
pub(crate) async fn delete_provider_secret(
    state: tauri::State<'_, ManagementState>,
    secret_ref: String,
) -> Result<ProviderSecretResponse, DesktopApiError> {
    let (management, runtime) = state.command().await?;
    management
        .delete_provider_secret(runtime.client(), &secret_ref)
        .await
}

#[tauri::command]
pub(crate) async fn list_sessions(
    state: tauri::State<'_, ManagementState>,
    query: SessionQuery,
) -> Result<SessionList, DesktopApiError> {
    let (management, runtime) = state.command().await?;
    management.list_sessions(runtime.client(), &query).await
}

#[tauri::command]
pub(crate) async fn session_messages(
    state: tauri::State<'_, ManagementState>,
    session_key: String,
    query: SessionMessageQuery,
) -> Result<SessionMessages, DesktopApiError> {
    let (management, runtime) = state.command().await?;
    management
        .session_messages(runtime.client(), &session_key, &query)
        .await
}

#[tauri::command]
pub(crate) async fn usage(
    state: tauri::State<'_, ManagementState>,
    query: UsageQuery,
) -> Result<UsageResponse, DesktopApiError> {
    let (management, runtime) = state.command().await?;
    management.usage(runtime.client(), &query).await
}

#[tauri::command]
pub(crate) async fn diagnostic_logs(
    state: tauri::State<'_, ManagementState>,
    query: DiagnosticLogQuery,
) -> Result<DiagnosticLogs, DesktopApiError> {
    let (management, runtime) = state.command().await?;
    management.diagnostic_logs(runtime.client(), &query).await
}

#[tauri::command]
pub(crate) async fn export_diagnostics(
    state: tauri::State<'_, ManagementState>,
    query: DiagnosticExportQuery,
) -> Result<DiagnosticExportReceipt, DesktopApiError> {
    let (management, runtime) = state.command().await?;
    management
        .export_diagnostics(runtime.client(), &query)
        .await
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{DesktopApiError, map_management_error, map_vault_error, write_diagnostic_export};
    use tempfile::tempdir;
    use wokrouter_storage::TokenVaultError;
    use wokrouter_wokcore_client::ManagementError;

    #[test]
    fn bridge_errors_are_stable_codes_without_paths_or_secrets() {
        let errors = [
            map_management_error(ManagementError::InvalidResponse),
            map_management_error(ManagementError::Unauthorized),
            map_vault_error(TokenVaultError::BackendFailure),
            DesktopApiError::initialization(),
        ];

        for error in errors {
            let encoded = serde_json::to_value(error).unwrap();
            assert_eq!(encoded.as_object().unwrap().len(), 1);
            assert!(encoded["code"].as_str().is_some());
            assert!(!encoded.to_string().contains(['\\', '/']));
            assert!(!encoded.to_string().contains("secret"));
        }
    }

    #[test]
    fn diagnostic_exports_return_only_a_bounded_receipt() {
        let destination = tempdir().unwrap();
        let receipt = write_diagnostic_export(destination.path(), b"synthetic zip").unwrap();

        assert_eq!(receipt.bytes, 13);
        assert!(receipt.file_name.starts_with("wokcore-diagnostics-"));
        assert!(receipt.file_name.ends_with(".zip"));
        assert_eq!(
            serde_json::to_value(&receipt).unwrap(),
            json!({
                "file_name": receipt.file_name,
                "bytes": 13
            })
        );
    }
}
