use secrecy::ExposeSecret;
use wokrouter_platform::{
    AppPaths, ClientIntegrationManager, ClientKind, ClientRoots, DoctorReport, DoctorSeverity,
    DoctorStatus, IntegrationDoctor, IntegrationStatus, RestoreResult,
};

use super::{CommandError, authorize, client, executable, load_token};

pub async fn integrate(paths: &AppPaths, client_kind: ClientKind) -> Result<u8, CommandError> {
    let manager = writable_manager(paths)?;
    let core = client(paths)?;
    let token = authorize(executable(paths)?).await?;
    if client_kind == ClientKind::Copilot {
        let setup = manager.copilot_setup(&core, &token).await?;
        println!(
            "{}",
            serde_json::to_string(&setup).expect("Copilot setup is serializable")
        );
    } else {
        manager.inject(client_kind, &core, &token).await?;
        println!(
            "WokRouter integration for {} is active.",
            client_kind.as_str()
        );
    }
    Ok(0)
}

pub async fn restore(paths: &AppPaths, client_kind: ClientKind) -> Result<u8, CommandError> {
    let manager = writable_manager(paths)?;
    let core = client(paths)?;
    let token = authorize(executable(paths)?).await?;
    match manager.restore(client_kind, &core, &token).await? {
        RestoreResult::Restored => println!(
            "WokRouter integration for {} was restored.",
            client_kind.as_str()
        ),
        RestoreResult::AlreadyRestored => println!(
            "WokRouter integration for {} is already restored.",
            client_kind.as_str()
        ),
        RestoreResult::ManualActionRequired => println!(
            "The WokCore token was revoked. Remove the WokCore BYOK provider and saved API key from GitHub Copilot App to finish restoration."
        ),
        RestoreResult::Conflict { .. } => return Err(CommandError::ClientConflict),
    }
    Ok(0)
}

pub async fn doctor(paths: &AppPaths, json: bool) -> Result<u8, CommandError> {
    let manager = read_only_manager(paths)?;
    let local_report = IntegrationDoctor::inspect(&manager)?;
    let report = if local_report
        .checks
        .iter()
        .any(|check| check.id.ends_with("_token"))
    {
        let core = client(paths)?;
        let management_token = load_token().await?;
        IntegrationDoctor::inspect_with_runtime(&manager, &core, management_token.as_ref()).await?
    } else {
        local_report
    };
    render_doctor(&report, json);
    Ok(0)
}

pub async fn repair(paths: &AppPaths, check_id: &str) -> Result<u8, CommandError> {
    let client_kind = repair_client(check_id).ok_or(CommandError::Usage)?;
    let core = client(paths)?;
    let token = authorize(executable(paths)?).await?;
    let report =
        IntegrationDoctor::inspect_with_runtime(&read_only_manager(paths)?, &core, Some(&token))
            .await?;
    match repair_decision(&report, check_id)? {
        RepairDecision::AlreadyHealthy => {
            println!("Doctor check {check_id} is already healthy.");
            return Ok(0);
        }
        RepairDecision::Run => {}
    }
    let manager = writable_manager(paths)?;
    let status = manager.repair(client_kind, &core, &token).await?;
    if !matches!(status, IntegrationStatus::Injected { .. }) {
        return Err(CommandError::ClientOperation);
    }
    if client_kind == ClientKind::Copilot {
        let setup = manager.copilot_setup(&core, &token).await?;
        println!(
            "Run api_key_command locally and paste its output into the GitHub Copilot App API key field: {}",
            serde_json::to_string(&setup).expect("Copilot setup is serializable")
        );
        return Ok(0);
    }
    println!("Doctor repair {check_id} completed.");
    Ok(0)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RepairDecision {
    AlreadyHealthy,
    Run,
}

fn repair_decision(report: &DoctorReport, check_id: &str) -> Result<RepairDecision, CommandError> {
    let check = report
        .checks
        .iter()
        .find(|check| check.id == check_id)
        .ok_or(CommandError::ClientNotInstalled)?;
    if check.status == DoctorStatus::Healthy {
        return Ok(RepairDecision::AlreadyHealthy);
    }
    let expected = format!("doctor --repair {check_id}");
    if check.remediation.as_deref() == Some(expected.as_str()) {
        Ok(RepairDecision::Run)
    } else {
        Err(CommandError::ClientConflict)
    }
}

pub fn integration_token(paths: &AppPaths, client_kind: ClientKind) -> Result<u8, CommandError> {
    let manager = read_only_manager(paths)?;
    if !matches!(
        manager.status(client_kind)?,
        IntegrationStatus::Injected { .. }
    ) {
        return Err(CommandError::ClientConflict);
    }
    let token = manager.read_token(client_kind)?;
    println!("{}", token.expose_secret());
    Ok(0)
}

pub fn repair_client(check_id: &str) -> Option<ClientKind> {
    match check_id {
        "codex_config" | "codex_token" => Some(ClientKind::Codex),
        "claude_config" | "claude_token" => Some(ClientKind::Claude),
        "copilot_config" | "copilot_token" => Some(ClientKind::Copilot),
        _ => None,
    }
}

fn writable_manager(paths: &AppPaths) -> Result<ClientIntegrationManager, CommandError> {
    let roots = ClientRoots::discover()?;
    let token_command = std::env::current_exe().map_err(|_| CommandError::ClientOperation)?;
    ClientIntegrationManager::new(roots, paths.integration_dir.clone(), token_command)
        .map_err(Into::into)
}

fn read_only_manager(paths: &AppPaths) -> Result<ClientIntegrationManager, CommandError> {
    ClientIntegrationManager::open_read_only(
        ClientRoots::discover()?,
        paths.integration_dir.clone(),
    )
    .map_err(Into::into)
}

fn render_doctor(report: &DoctorReport, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string(report).expect("doctor report is serializable")
        );
        return;
    }
    for check in &report.checks {
        let remediation = check
            .remediation
            .as_deref()
            .map(|command| format!("; remediation: wokrouter {command}"))
            .unwrap_or_default();
        println!(
            "{}: {} ({}){}",
            check.id,
            doctor_status(check.status),
            doctor_severity(check.severity),
            remediation
        );
    }
}

const fn doctor_status(status: DoctorStatus) -> &'static str {
    match status {
        DoctorStatus::Healthy => "healthy",
        DoctorStatus::Missing => "missing",
        DoctorStatus::Drifted => "drifted",
        DoctorStatus::Conflict => "conflict",
        DoctorStatus::Unsupported => "unsupported",
    }
}

const fn doctor_severity(severity: DoctorSeverity) -> &'static str {
    match severity {
        DoctorSeverity::Info => "info",
        DoctorSeverity::Warning => "warning",
        DoctorSeverity::Error => "error",
    }
}

#[cfg(test)]
mod tests {
    use wokrouter_platform::{DoctorCheck, DoctorReport, DoctorSeverity, DoctorStatus};

    use super::{RepairDecision, repair_decision};
    use crate::commands::CommandError;

    #[test]
    fn named_repair_runs_only_the_reported_remediation() {
        let report = DoctorReport {
            schema_version: 1,
            checks: vec![
                DoctorCheck {
                    id: "codex_config".to_owned(),
                    severity: DoctorSeverity::Info,
                    status: DoctorStatus::Healthy,
                    summary_key: "healthy".to_owned(),
                    remediation: None,
                },
                DoctorCheck {
                    id: "codex_token".to_owned(),
                    severity: DoctorSeverity::Error,
                    status: DoctorStatus::Missing,
                    summary_key: "missing".to_owned(),
                    remediation: Some("doctor --repair codex_token".to_owned()),
                },
            ],
        };

        assert_eq!(
            repair_decision(&report, "codex_config").unwrap(),
            RepairDecision::AlreadyHealthy
        );
        assert_eq!(
            repair_decision(&report, "codex_token").unwrap(),
            RepairDecision::Run
        );
        assert_eq!(
            repair_decision(&report, "claude_token").unwrap_err(),
            CommandError::ClientNotInstalled
        );
    }
}
