mod authorize;
mod clients;
mod diagnostics;
mod discovery;
mod error;
mod http;
mod management;
mod model;
mod providers;
mod service;
mod sessions;
mod usage;

use std::{
    fmt,
    num::NonZeroU32,
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use discovery::DiscoveryRead;
use http::{CapabilitiesWire, HealthWire, HttpError, WokCoreHttp};
use semver::Version;
use uuid::Uuid;

pub use authorize::{AuthorizationError, AuthorizationState, WokCoreAuthorizer};
pub use clients::{IntegrationRuntime, IssuedProxyToken};
pub use diagnostics::{
    DiagnosticExportQuery, DiagnosticLogLevel, DiagnosticLogQuery, DiagnosticLogs, DiagnosticOrder,
};
pub use error::ClientError;
pub use management::ManagementError;
pub use model::{Compatibility, CoreConnection, CoreHandshake};
pub use providers::{
    EndpointPolicy, ModelAlias, ModelSource, ProviderAccount, ProviderAccountAuth, ProviderAdapter,
    ProviderAuthKind, ProviderCandidate, ProviderCapabilities, ProviderCatalogResponse,
    ProviderCommitRequest, ProviderCommitResponse, ProviderConfig, ProviderDefinition,
    ProviderInstance, ProviderModelsResponse, ProviderReloadStatus, ProviderRuntimeResponse,
    ProviderSecretCreate, ProviderSecretOperation, ProviderSecretPurpose, ProviderSecretResponse,
    ProviderValidationResponse, PublicModel, RouteRule, RouteTarget, RoutingConfig,
};
pub use service::{ServiceError, ServicePhase, ServiceStatus};
pub use sessions::{
    IndexPhase, IndexStatus, MessageRole, SessionAvailability, SessionList, SessionListItem,
    SessionMessage, SessionMessageQuery, SessionMessages, SessionQuery, SessionSource,
    SourceAvailability, SourceIndexStatus,
};
pub use usage::{UsageBucket, UsageGroup, UsageQuery, UsageResponse, UsageTotals};

const SUPPORTED_API_MAJOR: u32 = 1;

#[derive(Clone)]
pub struct WokCoreClient {
    discovery_file: PathBuf,
    http: WokCoreHttp,
    runtime_policy: RuntimePolicy,
}

pub type WokCoreRuntimeValidator = dyn Fn(NonZeroU32) -> bool + Send + Sync;

#[derive(Clone)]
enum RuntimePolicy {
    Unrestricted,
    Fixed {
        identity: WokCoreRuntimeIdentity,
        validator: Arc<WokCoreRuntimeValidator>,
    },
    PendingTrustedExecutable(Arc<OnceLock<Arc<WokCoreRuntimeValidator>>>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WokCoreRuntimeIdentity {
    process_id: NonZeroU32,
    instance_id: Uuid,
}

impl WokCoreRuntimeIdentity {
    pub fn process_id(self) -> NonZeroU32 {
        self.process_id
    }
}

#[derive(Clone)]
pub struct WokCoreRuntimeBinder {
    validator: Arc<OnceLock<Arc<WokCoreRuntimeValidator>>>,
}

impl WokCoreRuntimeBinder {
    pub fn bind_trusted_executable(&self, validator: Arc<WokCoreRuntimeValidator>) -> bool {
        self.validator.set(validator).is_ok() || self.validator.get().is_some()
    }
}

impl fmt::Debug for WokCoreRuntimeBinder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WokCoreRuntimeBinder")
            .finish_non_exhaustive()
    }
}

impl WokCoreClient {
    pub fn new(discovery_file: impl Into<PathBuf>) -> Result<Self, ClientError> {
        Ok(Self {
            discovery_file: discovery_file.into(),
            http: WokCoreHttp::new()?,
            runtime_policy: RuntimePolicy::Unrestricted,
        })
    }

    pub fn discovered_runtime_identity(&self) -> Option<WokCoreRuntimeIdentity> {
        match discovery::read(&self.discovery_file) {
            DiscoveryRead::Record(record) => Some(WokCoreRuntimeIdentity {
                process_id: record.process_id,
                instance_id: record.instance_id,
            }),
            DiscoveryRead::Missing | DiscoveryRead::Invalid => None,
        }
    }

    pub fn discovered_process_id(&self) -> Option<NonZeroU32> {
        self.discovered_runtime_identity()
            .map(WokCoreRuntimeIdentity::process_id)
    }

    pub fn bound_to_runtime(
        &self,
        identity: WokCoreRuntimeIdentity,
        validator: Arc<WokCoreRuntimeValidator>,
    ) -> Self {
        Self {
            discovery_file: self.discovery_file.clone(),
            http: self.http.clone(),
            runtime_policy: RuntimePolicy::Fixed {
                identity,
                validator,
            },
        }
    }

    pub fn pending_trusted_executable_runtime(&self) -> (Self, WokCoreRuntimeBinder) {
        let validator = Arc::new(OnceLock::new());
        (
            Self {
                discovery_file: self.discovery_file.clone(),
                http: self.http.clone(),
                runtime_policy: RuntimePolicy::PendingTrustedExecutable(Arc::clone(&validator)),
            },
            WokCoreRuntimeBinder { validator },
        )
    }

    pub async fn connection(&self) -> CoreConnection {
        let discovery = match self.read_discovery() {
            DiscoveryRead::Missing => return CoreConnection::Missing,
            DiscoveryRead::Invalid => return CoreConnection::InvalidRuntime,
            DiscoveryRead::Record(record) => record,
        };
        if discovery.api_major != SUPPORTED_API_MAJOR {
            return CoreConnection::Incompatible(Compatibility::for_discovered_major(
                discovery.api_major,
                SUPPORTED_API_MAJOR,
            ));
        }

        let health = match self.http.health(&discovery).await {
            Ok(health) => health,
            Err(HttpError::Transport) => return CoreConnection::Stopped,
            Err(HttpError::InvalidResponse) => return CoreConnection::InvalidRuntime,
        };
        if !valid_health(&health, discovery.instance_id) {
            return CoreConnection::InvalidRuntime;
        }
        if !self.runtime_authorized(&discovery) {
            return CoreConnection::Missing;
        }

        let capabilities = match self.http.capabilities(&discovery).await {
            Ok(capabilities) => capabilities,
            Err(HttpError::Transport) => return CoreConnection::Stopped,
            Err(HttpError::InvalidResponse) => return CoreConnection::InvalidRuntime,
        };
        connection_from_capabilities(discovery, capabilities)
    }

    fn read_discovery(&self) -> DiscoveryRead {
        if matches!(
            &self.runtime_policy,
            RuntimePolicy::PendingTrustedExecutable(validator) if validator.get().is_none()
        ) {
            return DiscoveryRead::Missing;
        }
        match discovery::read(&self.discovery_file) {
            DiscoveryRead::Record(record) if !self.runtime_authorized(&record) => {
                DiscoveryRead::Missing
            }
            other => other,
        }
    }

    pub(crate) fn runtime_authorized(&self, record: &discovery::ValidatedDiscovery) -> bool {
        match &self.runtime_policy {
            RuntimePolicy::Unrestricted => true,
            RuntimePolicy::Fixed {
                identity,
                validator,
            } => {
                identity.process_id == record.process_id
                    && identity.instance_id == record.instance_id
                    && validator(record.process_id)
            }
            RuntimePolicy::PendingTrustedExecutable(validator) => validator
                .get()
                .is_some_and(|validator| validator(record.process_id)),
        }
    }

    pub(crate) fn has_runtime_policy(&self) -> bool {
        !matches!(self.runtime_policy, RuntimePolicy::Unrestricted)
    }
}

impl fmt::Debug for WokCoreClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WokCoreClient")
            .finish_non_exhaustive()
    }
}

fn valid_health(health: &HealthWire, expected_instance_id: Uuid) -> bool {
    health.status == "ok"
        && Uuid::parse_str(&health.instance_id).is_ok_and(|id| id == expected_instance_id)
}

fn connection_from_capabilities(
    discovery: discovery::ValidatedDiscovery,
    capabilities: CapabilitiesWire,
) -> CoreConnection {
    let valid_identity =
        Uuid::parse_str(&capabilities.instance_id).is_ok_and(|id| id == discovery.instance_id);
    let valid_installation_id = capabilities
        .installation_id
        .as_deref()
        .is_none_or(valid_opaque_identity);
    let valid_version = Version::parse(&capabilities.wokcore_version)
        .is_ok_and(|version| version == discovery.wokcore_version);
    let valid_range = capabilities.minimum_management_api_major > 0
        && capabilities.minimum_management_api_major <= capabilities.maximum_management_api_major
        && capabilities.management_api_major >= capabilities.minimum_management_api_major
        && capabilities.management_api_major <= capabilities.maximum_management_api_major;
    if !valid_identity || !valid_installation_id || !valid_version || !valid_range {
        return CoreConnection::InvalidRuntime;
    }

    let compatibility = Compatibility {
        wokcore_minimum_api_major: capabilities.minimum_management_api_major,
        wokcore_maximum_api_major: capabilities.maximum_management_api_major,
        wokrouter_minimum_api_major: SUPPORTED_API_MAJOR,
        wokrouter_maximum_api_major: SUPPORTED_API_MAJOR,
    };
    if !compatibility.overlaps() {
        return CoreConnection::Incompatible(compatibility);
    }
    if capabilities.management_api_major != discovery.api_major
        || capabilities.management_api_major != SUPPORTED_API_MAJOR
        || !valid_string_set(&capabilities.provider_protocols)
        || !valid_string_set(&capabilities.capabilities)
    {
        return CoreConnection::InvalidRuntime;
    }

    CoreConnection::Running(CoreHandshake {
        instance_id: discovery.instance_id.to_string(),
        installation_id: capabilities.installation_id,
        version: capabilities.wokcore_version,
        management_api_major: capabilities.management_api_major,
        provider_protocols: capabilities.provider_protocols.into_iter().collect(),
        capabilities: capabilities.capabilities.into_iter().collect(),
    })
}

fn valid_opaque_identity(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_string_set(values: &[String]) -> bool {
    values.iter().all(|value| {
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    })
}
