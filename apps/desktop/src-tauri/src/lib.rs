//! WokRouter desktop shell.

mod control;

use control::{DaemonStatusDto, DesktopControl, DesktopControlError};

struct DesktopState {
    control: Result<DesktopControl, DesktopControlError>,
}

impl DesktopState {
    fn new(control: Result<DesktopControl, DesktopControlError>) -> Self {
        Self { control }
    }
}

async fn daemon_status_for(state: &DesktopState) -> Result<DaemonStatusDto, String> {
    let control = state.control.as_ref().map_err(|error| error.to_string())?;
    control.status().await.map_err(|error| error.to_string())
}

async fn start_daemon_for(state: &DesktopState) -> Result<(), String> {
    let control = state.control.as_ref().map_err(|error| error.to_string())?;
    control.start().await.map_err(|error| error.to_string())
}

#[tauri::command]
async fn daemon_status(state: tauri::State<'_, DesktopState>) -> Result<DaemonStatusDto, String> {
    daemon_status_for(&state).await
}

#[tauri::command]
async fn start_daemon(state: tauri::State<'_, DesktopState>) -> Result<(), String> {
    start_daemon_for(&state).await
}

pub fn run() -> tauri::Result<()> {
    let state = DesktopState::new(DesktopControl::discover());
    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![daemon_status, start_daemon])
        .run(tauri::generate_context!())
}

#[cfg(test)]
mod tests {
    use super::{DesktopState, daemon_status_for, start_daemon_for};
    use crate::control::DesktopControlError;

    #[tokio::test]
    async fn discovery_failure_maps_to_safe_recoverable_commands() {
        let state = DesktopState::new(Err(DesktopControlError::Initialization));

        let status_error = daemon_status_for(&state).await.unwrap_err();
        let start_error = start_daemon_for(&state).await.unwrap_err();

        assert_eq!(status_error, "Unable to initialize desktop control.");
        assert_eq!(start_error, "Unable to initialize desktop control.");
        assert!(!status_error.contains(['\\', '/']));
        assert!(!start_error.contains(['\\', '/']));
    }
}
