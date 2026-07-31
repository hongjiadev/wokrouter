use std::{collections::BTreeSet, path::PathBuf};

use secrecy::SecretString;
use serde::Serialize;
use wokrouter_platform::IntegrationError;
use wokrouter_platform::{
    AppPaths, PlatformError, SelectedWokCoreRuntime, WokCoreRuntimeChannel,
    discover_wokcore_executable,
};
use wokrouter_storage::{NativeWokCoreTokenVault, TokenVaultError, WokCoreTokenVault};
use wokrouter_wokcore_client::{
    AuthorizationError, CoreConnection, CoreHandshake, ServiceError, ServicePhase, ServiceStatus,
    WokCoreAuthorizer, WokCoreClient,
};

pub mod integrations;
pub mod start;
pub mod status;
pub mod stop;

pub const NOT_RUNNING_EXIT_CODE: u8 = 3;
pub const AUTHORIZATION_REQUIRED_EXIT_CODE: u8 = 4;

pub(crate) trait CommandRuntime {
    fn channel(&self) -> WokCoreRuntimeChannel;
    fn executable(&self) -> Option<&std::path::Path>;
    fn client(&self) -> &WokCoreClient;
    fn establish_production_binding(&self, executable: &std::path::Path) -> bool;
    async fn connection(&self) -> CoreConnection;
}

impl CommandRuntime for SelectedWokCoreRuntime {
    fn channel(&self) -> WokCoreRuntimeChannel {
        self.channel()
    }

    fn executable(&self) -> Option<&std::path::Path> {
        self.executable()
    }

    fn client(&self) -> &WokCoreClient {
        self.client()
    }

    fn establish_production_binding(&self, executable: &std::path::Path) -> bool {
        self.establish_production_binding(executable)
    }

    async fn connection(&self) -> CoreConnection {
        self.connection().await
    }
}

pub fn client(paths: &AppPaths) -> Result<WokCoreClient, CommandError> {
    WokCoreClient::new(&paths.wokcore_discovery_file).map_err(CommandError::from)
}

pub fn executable(paths: &AppPaths) -> Result<PathBuf, CommandError> {
    discover_wokcore_executable(&paths.wokcore_install_record)?.ok_or(CommandError::WokCoreMissing)
}

pub async fn authorize(executable: PathBuf) -> Result<SecretString, CommandError> {
    let vault = NativeWokCoreTokenVault::new();
    if let Some(token) = vault.load().await? {
        return Ok(token);
    }
    authorize_fresh(&vault, executable).await
}

pub async fn reauthorize(executable: PathBuf) -> Result<SecretString, CommandError> {
    let vault = NativeWokCoreTokenVault::new();
    vault.delete().await?;
    authorize_fresh(&vault, executable).await
}

async fn authorize_fresh(
    vault: &NativeWokCoreTokenVault,
    executable: PathBuf,
) -> Result<SecretString, CommandError> {
    let token = WokCoreAuthorizer::new(executable).authorize().await?;
    vault.store(clone_secret(&token)).await?;
    Ok(token)
}

pub async fn load_token() -> Result<Option<SecretString>, CommandError> {
    NativeWokCoreTokenVault::new()
        .load()
        .await
        .map_err(CommandError::from)
}

fn clone_secret(token: &SecretString) -> SecretString {
    use secrecy::ExposeSecret;

    SecretString::from(token.expose_secret().to_owned())
}

pub fn public_status(
    runtime_channel: WokCoreRuntimeChannel,
    connection: CoreConnection,
) -> CoreStatus {
    match connection {
        CoreConnection::Missing => {
            CoreStatus::bare(runtime_channel, CoreUiState::Stopped, "not_running")
        }
        CoreConnection::Stopped => {
            CoreStatus::bare(runtime_channel, CoreUiState::Stopped, "not_running")
        }
        CoreConnection::Incompatible(_) => {
            CoreStatus::bare(runtime_channel, CoreUiState::Incompatible, "incompatible")
        }
        CoreConnection::InvalidRuntime => CoreStatus::bare(
            runtime_channel,
            CoreUiState::InvalidRuntime,
            "invalid_runtime",
        ),
        CoreConnection::Running(handshake) => CoreStatus::from_handshake(
            runtime_channel,
            CoreUiState::AuthorizationRequired,
            handshake,
        ),
    }
}

pub fn protected_status(
    runtime_channel: WokCoreRuntimeChannel,
    handshake: CoreHandshake,
    service: ServiceStatus,
) -> CoreStatus {
    let state = match service.phase {
        ServicePhase::Starting => CoreUiState::Starting,
        ServicePhase::Running => CoreUiState::Running,
        ServicePhase::Draining | ServicePhase::AwaitingCancellation => CoreUiState::Draining,
        ServicePhase::Stopping => CoreUiState::Stopped,
    };
    let mut status = CoreStatus::from_handshake(runtime_channel, state, handshake);
    status.phase = Some(service.phase);
    status.active_requests = Some(service.active_requests);
    status.error_code = None;
    status
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreUiState {
    Missing,
    Stopped,
    Starting,
    Running,
    Draining,
    AuthorizationRequired,
    Incompatible,
    InvalidRuntime,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CoreStatus {
    pub state: CoreUiState,
    pub runtime_channel: WokCoreRuntimeChannel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub management_api_major: Option<u32>,
    pub capabilities: BTreeSet<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<ServicePhase>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_requests: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<&'static str>,
}

impl CoreStatus {
    pub fn missing(runtime_channel: WokCoreRuntimeChannel) -> Self {
        Self::bare(runtime_channel, CoreUiState::Missing, "missing")
    }

    fn bare(
        runtime_channel: WokCoreRuntimeChannel,
        state: CoreUiState,
        error_code: &'static str,
    ) -> Self {
        Self {
            state,
            runtime_channel,
            version: None,
            management_api_major: None,
            capabilities: BTreeSet::new(),
            phase: None,
            active_requests: None,
            error_code: Some(error_code),
        }
    }

    fn from_handshake(
        runtime_channel: WokCoreRuntimeChannel,
        state: CoreUiState,
        handshake: CoreHandshake,
    ) -> Self {
        Self {
            state,
            runtime_channel,
            version: Some(handshake.version),
            management_api_major: Some(handshake.management_api_major),
            capabilities: handshake.capabilities,
            phase: None,
            active_requests: None,
            error_code: Some("authorization_required"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum CommandError {
    #[error(
        "usage: wokrouter <start|status [--json]|stop|integrate <client>|restore <client>|doctor [--json|--repair <check-id>]|integration-token <client>>"
    )]
    Usage,
    #[error("WokCore is not installed or is not available on PATH")]
    WokCoreMissing,
    #[error("WokCore did not become ready within five seconds")]
    StartTimedOut,
    #[error("WokCore failed to start")]
    StartFailed,
    #[error("WokCore did not stop within five seconds")]
    StopTimedOut,
    #[error("WokCore client authorization is required")]
    AuthorizationRequired,
    #[error("the native WokCore token credential service is unavailable")]
    CredentialServiceUnavailable,
    #[error("WokCore local control failed")]
    CoreControl,
    #[error("WokCore runtime metadata is invalid")]
    InvalidRuntime,
    #[error("WokCore API version is incompatible")]
    Incompatible,
    #[error("the development WokCore runtime is managed by the IDE")]
    DevelopmentRuntimeManagedByIde,
    #[error("the client home directory is unavailable")]
    ClientHomeUnavailable,
    #[error("the client integration is not installed")]
    ClientNotInstalled,
    #[error("the client integration is unsupported by this WokCore runtime")]
    ClientUnsupported,
    #[error("the client configuration has an ownership conflict")]
    ClientConflict,
    #[error("the client integration belongs to a different WokCore installation")]
    ClientRuntimeChanged,
    #[error("the client integration state is invalid")]
    ClientInvalidState,
    #[error("the client integration operation failed")]
    ClientOperation,
}

impl From<PlatformError> for CommandError {
    fn from(error: PlatformError) -> Self {
        match error {
            PlatformError::InvalidWokCoreInstallRecord => Self::InvalidRuntime,
            _ => Self::CoreControl,
        }
    }
}

impl From<wokrouter_wokcore_client::ClientError> for CommandError {
    fn from(_: wokrouter_wokcore_client::ClientError) -> Self {
        Self::CoreControl
    }
}

impl From<TokenVaultError> for CommandError {
    fn from(error: TokenVaultError) -> Self {
        match error {
            TokenVaultError::Unavailable => Self::CredentialServiceUnavailable,
            TokenVaultError::BackendFailure | TokenVaultError::InvalidEncoding => Self::CoreControl,
        }
    }
}

impl From<AuthorizationError> for CommandError {
    fn from(error: AuthorizationError) -> Self {
        match error {
            AuthorizationError::Unavailable => Self::WokCoreMissing,
            AuthorizationError::TimedOut
            | AuthorizationError::OutputTooLarge
            | AuthorizationError::Failed
            | AuthorizationError::InvalidResponse => Self::CoreControl,
        }
    }
}

impl From<ServiceError> for CommandError {
    fn from(error: ServiceError) -> Self {
        match error {
            ServiceError::Missing | ServiceError::Stopped => Self::CoreControl,
            ServiceError::Incompatible => Self::Incompatible,
            ServiceError::InvalidRuntime | ServiceError::InvalidResponse => Self::InvalidRuntime,
            ServiceError::Unauthorized | ServiceError::Forbidden => Self::AuthorizationRequired,
        }
    }
}

impl From<IntegrationError> for CommandError {
    fn from(error: IntegrationError) -> Self {
        match error {
            IntegrationError::MissingHome => Self::ClientHomeUnavailable,
            IntegrationError::NotInstalled => Self::ClientNotInstalled,
            IntegrationError::Unsupported => Self::ClientUnsupported,
            IntegrationError::Conflict => Self::ClientConflict,
            IntegrationError::RuntimeChanged => Self::ClientRuntimeChanged,
            IntegrationError::InvalidConfig | IntegrationError::InvalidState => {
                Self::ClientInvalidState
            }
            IntegrationError::Operation => Self::ClientOperation,
        }
    }
}
