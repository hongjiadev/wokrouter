//! WokRouter desktop shell.

mod control;
mod wokcore;

use control::{DesktopControl, DesktopControlError};
use wokcore::ManagementState;
use wokrouter_cli::commands::CoreStatus;

struct DesktopState {
    control: Result<DesktopControl, DesktopControlError>,
}

impl DesktopState {
    fn new(control: Result<DesktopControl, DesktopControlError>) -> Self {
        Self { control }
    }
}

async fn core_status_for(state: &DesktopState) -> Result<CoreStatus, String> {
    let control = state.control.as_ref().map_err(|error| error.to_string())?;
    control.status().await.map_err(|error| error.to_string())
}

async fn start_core_for(state: &DesktopState) -> Result<(), String> {
    let control = state.control.as_ref().map_err(|error| error.to_string())?;
    control.start().await.map_err(|error| error.to_string())
}

async fn stop_core_for(state: &DesktopState) -> Result<(), String> {
    let control = state.control.as_ref().map_err(|error| error.to_string())?;
    control.stop().await.map_err(|error| error.to_string())
}

#[tauri::command]
async fn core_status(state: tauri::State<'_, DesktopState>) -> Result<CoreStatus, String> {
    core_status_for(&state).await
}

#[tauri::command]
async fn start_core(state: tauri::State<'_, DesktopState>) -> Result<(), String> {
    start_core_for(&state).await
}

#[tauri::command]
async fn stop_core(state: tauri::State<'_, DesktopState>) -> Result<(), String> {
    stop_core_for(&state).await
}

pub fn run() -> tauri::Result<()> {
    let state = DesktopState::new(DesktopControl::discover());
    tauri::Builder::default()
        .manage(state)
        .manage(ManagementState::discover())
        .invoke_handler(tauri::generate_handler![
            core_status,
            start_core,
            stop_core,
            wokcore::provider_catalog,
            wokcore::provider_runtime,
            wokcore::provider_models,
            wokcore::validate_provider_config,
            wokcore::commit_provider_config,
            wokcore::reload_providers,
            wokcore::create_provider_secret,
            wokcore::replace_provider_secret,
            wokcore::delete_provider_secret,
            wokcore::list_sessions,
            wokcore::session_messages,
            wokcore::usage,
            wokcore::diagnostic_logs,
            wokcore::export_diagnostics,
        ])
        .run(tauri::generate_context!())
}

#[cfg(test)]
mod tests {
    use super::{DesktopState, core_status_for, start_core_for, stop_core_for};
    use crate::control::DesktopControlError;

    #[tokio::test]
    async fn discovery_failure_maps_to_safe_recoverable_commands() {
        let state = DesktopState::new(Err(DesktopControlError::Initialization));

        let status_error = core_status_for(&state).await.unwrap_err();
        let start_error = start_core_for(&state).await.unwrap_err();
        let stop_error = stop_core_for(&state).await.unwrap_err();

        assert_eq!(status_error, "Unable to initialize WokCore control.");
        assert_eq!(start_error, "Unable to initialize WokCore control.");
        assert_eq!(stop_error, "Unable to initialize WokCore control.");
        assert!(!status_error.contains(['\\', '/']));
        assert!(!start_error.contains(['\\', '/']));
        assert!(!stop_error.contains(['\\', '/']));
    }
}
