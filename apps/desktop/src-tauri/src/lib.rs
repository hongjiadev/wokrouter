//! WokRouter desktop shell.

mod control;
mod runtime;
mod wokcore;

use std::sync::Arc;

use control::DesktopControl;
use runtime::DesktopRuntimeState;
use wokcore::ManagementState;
use wokrouter_cli::commands::CoreStatus;

struct DesktopState {
    control: DesktopControl,
}

impl DesktopState {
    fn new(control: DesktopControl) -> Self {
        Self { control }
    }
}

async fn core_status_for(state: &DesktopState) -> Result<CoreStatus, String> {
    state
        .control
        .status()
        .await
        .map_err(|error| error.to_string())
}

async fn start_core_for(state: &DesktopState) -> Result<(), String> {
    state
        .control
        .start()
        .await
        .map_err(|error| error.to_string())
}

async fn stop_core_for(state: &DesktopState) -> Result<(), String> {
    state
        .control
        .stop()
        .await
        .map_err(|error| error.to_string())
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
    let runtime = Arc::new(DesktopRuntimeState::discover());
    let state = DesktopState::new(DesktopControl::new(runtime.clone()));
    tauri::Builder::default()
        .manage(state)
        .manage(ManagementState::discover(runtime))
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
