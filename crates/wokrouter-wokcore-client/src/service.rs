use std::time::Duration;

use reqwest::Method;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::{
    SUPPORTED_API_MAJOR, WokCoreClient,
    discovery::{DiscoveryRead, ValidatedDiscovery},
    http::ProtectedHttpError,
};

const STATUS_TIMEOUT: Duration = Duration::from_secs(3);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(35);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const CANCEL_TIMEOUT: Duration = Duration::from_secs(2);

impl WokCoreClient {
    pub async fn service_status(
        &self,
        token: &SecretString,
    ) -> Result<ServiceStatus, ServiceError> {
        let discovery = self.service_discovery()?;
        let response = self
            .http
            .protected_json::<LifecycleWire>(
                &discovery,
                Method::GET,
                "/wokcore/v1/service/status",
                token,
                STATUS_TIMEOUT,
            )
            .await
            .map_err(map_http_error)?;
        ServiceStatus::try_from(response)
    }

    pub async fn stop(&self, token: &SecretString) -> Result<(), ServiceError> {
        let discovery = self.service_discovery()?;
        let drained = self
            .lifecycle_request(
                &discovery,
                token,
                "/wokcore/v1/service/drain",
                DRAIN_TIMEOUT,
            )
            .await;
        let drained = match drained {
            Ok(status) if status.phase == ServicePhase::Draining && status.active_requests == 0 => {
                status
            }
            Ok(_) => {
                self.cancel_drain(&discovery, token).await;
                return Err(ServiceError::InvalidResponse);
            }
            Err(error) => {
                self.cancel_drain(&discovery, token).await;
                return Err(error);
            }
        };
        let _ = drained;

        match self
            .lifecycle_request(&discovery, token, "/wokcore/v1/service/stop", STOP_TIMEOUT)
            .await
        {
            Ok(status) if status.phase == ServicePhase::Stopping && status.active_requests == 0 => {
                Ok(())
            }
            Ok(_) => {
                self.cancel_drain(&discovery, token).await;
                Err(ServiceError::InvalidResponse)
            }
            Err(error) => {
                self.cancel_drain(&discovery, token).await;
                Err(error)
            }
        }
    }

    fn service_discovery(&self) -> Result<ValidatedDiscovery, ServiceError> {
        match self.read_discovery() {
            DiscoveryRead::Missing => Err(ServiceError::Missing),
            DiscoveryRead::Invalid => Err(ServiceError::InvalidRuntime),
            DiscoveryRead::Record(discovery) if discovery.api_major == SUPPORTED_API_MAJOR => {
                Ok(discovery)
            }
            DiscoveryRead::Record(_) => Err(ServiceError::Incompatible),
        }
    }

    async fn lifecycle_request(
        &self,
        discovery: &ValidatedDiscovery,
        token: &SecretString,
        path: &str,
        timeout: Duration,
    ) -> Result<ServiceStatus, ServiceError> {
        let response = self
            .http
            .protected_json::<LifecycleWire>(discovery, Method::POST, path, token, timeout)
            .await
            .map_err(map_http_error)?;
        ServiceStatus::try_from(response)
    }

    async fn cancel_drain(&self, discovery: &ValidatedDiscovery, token: &SecretString) {
        let _ = self
            .http
            .protected_json::<LifecycleWire>(
                discovery,
                Method::POST,
                "/wokcore/v1/service/drain/cancel",
                token,
                CANCEL_TIMEOUT,
            )
            .await;
    }
}

fn map_http_error(error: ProtectedHttpError) -> ServiceError {
    match error {
        ProtectedHttpError::Transport => ServiceError::Stopped,
        ProtectedHttpError::Unauthorized => ServiceError::Unauthorized,
        ProtectedHttpError::Forbidden => ServiceError::Forbidden,
        ProtectedHttpError::Conflict
        | ProtectedHttpError::InvalidRequest
        | ProtectedHttpError::InvalidResponse => ServiceError::InvalidResponse,
    }
}

#[derive(Deserialize)]
struct LifecycleWire {
    phase: String,
    active_requests: usize,
}

impl TryFrom<LifecycleWire> for ServiceStatus {
    type Error = ServiceError;

    fn try_from(value: LifecycleWire) -> Result<Self, Self::Error> {
        Ok(Self {
            phase: ServicePhase::parse(&value.phase).ok_or(ServiceError::InvalidResponse)?,
            active_requests: value.active_requests,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServicePhase {
    Starting,
    Running,
    Draining,
    AwaitingCancellation,
    Stopping,
}

impl ServicePhase {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "starting" => Some(Self::Starting),
            "running" => Some(Self::Running),
            "draining" => Some(Self::Draining),
            "awaiting_cancellation" => Some(Self::AwaitingCancellation),
            "stopping" => Some(Self::Stopping),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ServiceStatus {
    pub phase: ServicePhase,
    pub active_requests: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ServiceError {
    #[error("WokCore runtime metadata is missing")]
    Missing,
    #[error("WokCore is stopped")]
    Stopped,
    #[error("WokCore API version is incompatible")]
    Incompatible,
    #[error("WokCore runtime metadata is invalid")]
    InvalidRuntime,
    #[error("WokCore client authorization is required")]
    Unauthorized,
    #[error("WokCore client authorization lacks the required scope")]
    Forbidden,
    #[error("WokCore returned an invalid service response")]
    InvalidResponse,
}
