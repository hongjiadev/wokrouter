mod discovery;
mod error;
mod http;
mod model;

use std::{fmt, path::PathBuf};

use discovery::DiscoveryRead;
use http::{CapabilitiesWire, HealthWire, HttpError, WokCoreHttp};
use semver::Version;
use uuid::Uuid;

pub use error::ClientError;
pub use model::{Compatibility, CoreConnection, CoreHandshake};

const SUPPORTED_API_MAJOR: u32 = 1;

#[derive(Clone)]
pub struct WokCoreClient {
    discovery_file: PathBuf,
    http: WokCoreHttp,
}

impl WokCoreClient {
    pub fn new(discovery_file: impl Into<PathBuf>) -> Result<Self, ClientError> {
        Ok(Self {
            discovery_file: discovery_file.into(),
            http: WokCoreHttp::new()?,
        })
    }

    pub async fn connection(&self) -> CoreConnection {
        let discovery = match discovery::read(&self.discovery_file) {
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

        let capabilities = match self.http.capabilities(&discovery).await {
            Ok(capabilities) => capabilities,
            Err(HttpError::Transport) => return CoreConnection::Stopped,
            Err(HttpError::InvalidResponse) => return CoreConnection::InvalidRuntime,
        };
        connection_from_capabilities(discovery, capabilities)
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
    let valid_version = Version::parse(&capabilities.wokcore_version)
        .is_ok_and(|version| version == discovery.wokcore_version);
    let valid_range = capabilities.minimum_management_api_major > 0
        && capabilities.minimum_management_api_major <= capabilities.maximum_management_api_major
        && capabilities.management_api_major >= capabilities.minimum_management_api_major
        && capabilities.management_api_major <= capabilities.maximum_management_api_major;
    if !valid_identity || !valid_version || !valid_range {
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
        version: capabilities.wokcore_version,
        management_api_major: capabilities.management_api_major,
        provider_protocols: capabilities.provider_protocols.into_iter().collect(),
        capabilities: capabilities.capabilities.into_iter().collect(),
    })
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
