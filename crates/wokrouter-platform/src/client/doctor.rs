use secrecy::SecretString;
use serde::Serialize;
use wokrouter_wokcore_client::WokCoreClient;

use super::integrations::{
    ClientIntegrationManager, ClientKind, IntegrationError, IntegrationStatus, RemoteInspection,
    RemoteRuntimeStatus,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorStatus {
    Healthy,
    Missing,
    Drifted,
    Conflict,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DoctorCheck {
    pub id: String,
    pub severity: DoctorSeverity,
    pub status: DoctorStatus,
    pub summary_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DoctorReport {
    pub schema_version: u32,
    pub checks: Vec<DoctorCheck>,
}

pub struct IntegrationDoctor;

impl IntegrationDoctor {
    pub fn inspect(manager: &ClientIntegrationManager) -> Result<DoctorReport, IntegrationError> {
        let mut checks = Vec::with_capacity(6);
        for client in [ClientKind::Codex, ClientKind::Claude, ClientKind::Copilot] {
            match manager.status(client) {
                Ok(status) => {
                    checks.push(integration_check(client, status.clone()));
                    if matches!(status, IntegrationStatus::Injected { .. }) {
                        checks.push(token_check(client, manager.read_token(client).is_ok()));
                    }
                }
                Err(error) => checks.push(integration_error_check(client, error)),
            }
        }
        Ok(DoctorReport {
            schema_version: 1,
            checks,
        })
    }

    pub async fn inspect_with_runtime(
        manager: &ClientIntegrationManager,
        core: &WokCoreClient,
        management_token: Option<&SecretString>,
    ) -> Result<DoctorReport, IntegrationError> {
        let mut report = Self::inspect(manager)?;
        for client in [ClientKind::Codex, ClientKind::Claude, ClientKind::Copilot] {
            if matches!(
                manager.status(client),
                Ok(IntegrationStatus::Injected { .. })
            ) {
                let inspection = manager.inspect_remote(client, core, management_token).await;
                if inspection.runtime == RemoteRuntimeStatus::Changed
                    && let Some(config) = report
                        .checks
                        .iter_mut()
                        .find(|check| check.id == format!("{}_config", client.as_str()))
                {
                    config.severity = DoctorSeverity::Error;
                    config.status = DoctorStatus::Drifted;
                    config.summary_key = format!("integration.{}.runtime_changed", client.as_str());
                    config.remediation =
                        Some(format!("doctor --repair {}_config", client.as_str()));
                }
                if matches!(
                    inspection.runtime,
                    RemoteRuntimeStatus::Healthy | RemoteRuntimeStatus::Changed
                ) && inspection.token_active == Some(false)
                    && let Some(token) = report
                        .checks
                        .iter_mut()
                        .find(|check| check.id == format!("{}_token", client.as_str()))
                {
                    *token = token_check(client, false);
                    token.summary_key = format!("integration.{}.token_revoked", client.as_str());
                }
                report
                    .checks
                    .push(runtime_check(client, inspection.runtime));
                report.checks.push(remote_token_check(client, inspection));
            }
        }
        Ok(report)
    }
}

fn runtime_check(client: ClientKind, status: RemoteRuntimeStatus) -> DoctorCheck {
    let (severity, doctor_status, suffix, remediation) = match status {
        RemoteRuntimeStatus::Healthy => {
            (DoctorSeverity::Info, DoctorStatus::Healthy, "healthy", None)
        }
        RemoteRuntimeStatus::Changed => (
            DoctorSeverity::Error,
            DoctorStatus::Drifted,
            "changed",
            Some(format!("doctor --repair {}_config", client.as_str())),
        ),
        RemoteRuntimeStatus::Missing => (
            DoctorSeverity::Error,
            DoctorStatus::Missing,
            "missing",
            None,
        ),
        RemoteRuntimeStatus::Unsupported => (
            DoctorSeverity::Error,
            DoctorStatus::Unsupported,
            "unsupported",
            None,
        ),
        RemoteRuntimeStatus::IdentityMismatch => (
            DoctorSeverity::Error,
            DoctorStatus::Conflict,
            "identity_mismatch",
            None,
        ),
        RemoteRuntimeStatus::Invalid => (
            DoctorSeverity::Error,
            DoctorStatus::Conflict,
            "invalid",
            None,
        ),
    };
    DoctorCheck {
        id: format!("{}_runtime", client.as_str()),
        severity,
        status: doctor_status,
        summary_key: format!("integration.{}.runtime_{suffix}", client.as_str()),
        remediation,
    }
}

fn remote_token_check(client: ClientKind, inspection: RemoteInspection) -> DoctorCheck {
    let (severity, status, suffix, remediation) =
        match (inspection.runtime, inspection.token_active) {
            (RemoteRuntimeStatus::Healthy, Some(true)) => {
                (DoctorSeverity::Info, DoctorStatus::Healthy, "healthy", None)
            }
            (RemoteRuntimeStatus::Changed, Some(true)) => {
                (DoctorSeverity::Info, DoctorStatus::Healthy, "healthy", None)
            }
            (RemoteRuntimeStatus::Healthy | RemoteRuntimeStatus::Changed, Some(false)) => (
                DoctorSeverity::Error,
                DoctorStatus::Missing,
                "revoked",
                Some(format!("doctor --repair {}_token", client.as_str())),
            ),
            (RemoteRuntimeStatus::Healthy | RemoteRuntimeStatus::Changed, None) => (
                DoctorSeverity::Warning,
                DoctorStatus::Missing,
                "unverified",
                None,
            ),
            _ => (
                DoctorSeverity::Error,
                DoctorStatus::Conflict,
                "unavailable",
                None,
            ),
        };
    DoctorCheck {
        id: format!("{}_token_remote", client.as_str()),
        severity,
        status,
        summary_key: format!("integration.{}.token_remote_{suffix}", client.as_str()),
        remediation,
    }
}

fn integration_error_check(client: ClientKind, error: IntegrationError) -> DoctorCheck {
    let error_name = match error {
        IntegrationError::InvalidConfig => "invalid_config",
        IntegrationError::InvalidState => "invalid_state",
        IntegrationError::RuntimeChanged => "runtime_changed",
        IntegrationError::MissingHome
        | IntegrationError::NotInstalled
        | IntegrationError::Unsupported
        | IntegrationError::Conflict
        | IntegrationError::Operation => "unavailable",
    };
    DoctorCheck {
        id: format!("{}_config", client.as_str()),
        severity: DoctorSeverity::Error,
        status: DoctorStatus::Conflict,
        summary_key: format!("integration.{}.{}", client.as_str(), error_name),
        remediation: None,
    }
}

fn integration_check(client: ClientKind, status: IntegrationStatus) -> DoctorCheck {
    let (severity, status, remediation) = match status {
        IntegrationStatus::Injected { .. } => (DoctorSeverity::Info, DoctorStatus::Healthy, None),
        IntegrationStatus::NotInstalled => (DoctorSeverity::Info, DoctorStatus::Missing, None),
        IntegrationStatus::Native => (
            DoctorSeverity::Warning,
            DoctorStatus::Missing,
            Some(format!("doctor --repair {}_config", client.as_str())),
        ),
        IntegrationStatus::Drifted => (
            DoctorSeverity::Error,
            DoctorStatus::Drifted,
            Some(format!("restore {}", client.as_str())),
        ),
        IntegrationStatus::Conflict => (
            DoctorSeverity::Error,
            DoctorStatus::Conflict,
            Some(format!("restore {}", client.as_str())),
        ),
        IntegrationStatus::Unsupported => {
            (DoctorSeverity::Warning, DoctorStatus::Unsupported, None)
        }
    };
    DoctorCheck {
        id: format!("{}_config", client.as_str()),
        severity,
        status,
        summary_key: format!(
            "integration.{}.{}",
            client.as_str(),
            doctor_status_name(status)
        ),
        remediation,
    }
}

fn token_check(client: ClientKind, healthy: bool) -> DoctorCheck {
    DoctorCheck {
        id: format!("{}_token", client.as_str()),
        severity: if healthy {
            DoctorSeverity::Info
        } else {
            DoctorSeverity::Error
        },
        status: if healthy {
            DoctorStatus::Healthy
        } else {
            DoctorStatus::Missing
        },
        summary_key: format!(
            "integration.{}.token_{}",
            client.as_str(),
            if healthy { "healthy" } else { "missing" }
        ),
        remediation: (!healthy).then(|| format!("doctor --repair {}_token", client.as_str())),
    }
}

const fn doctor_status_name(status: DoctorStatus) -> &'static str {
    match status {
        DoctorStatus::Healthy => "healthy",
        DoctorStatus::Missing => "missing",
        DoctorStatus::Drifted => "drifted",
        DoctorStatus::Conflict => "conflict",
        DoctorStatus::Unsupported => "unsupported",
    }
}
