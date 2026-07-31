//! WokRouter desktop shell.

mod control;
mod core_operation;
mod runtime;
mod wokcore;

use std::sync::Arc;

use control::DesktopControl;
use core_operation::{
    CoreOperationCoordinator, CoreOperationSnapshot, CoreUpdateCheck, EventFuture,
    OperationEventSink,
};
use runtime::DesktopRuntimeState;
use tauri::Emitter;
use wokcore::ManagementState;
use wokrouter_cli::commands::CoreStatus;

struct DesktopState {
    control: DesktopControl,
    core_operations: CoreOperationCoordinator,
}

impl DesktopState {
    fn new(control: DesktopControl, core_operations: CoreOperationCoordinator) -> Self {
        Self {
            control,
            core_operations,
        }
    }
}

struct TauriOperationEventSink {
    app: tauri::AppHandle,
}

impl OperationEventSink for TauriOperationEventSink {
    fn emit<'a>(&'a self, snapshot: &'a CoreOperationSnapshot) -> EventFuture<'a> {
        Box::pin(async move {
            let _ = self.app.emit("core-operation-progress", snapshot);
        })
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

async fn core_operation_status_for(
    state: &DesktopState,
) -> Result<Option<CoreOperationSnapshot>, String> {
    Ok(state.core_operations.status().await)
}

async fn install_and_start_core_for(
    app: tauri::AppHandle,
    state: &DesktopState,
) -> Result<CoreOperationSnapshot, String> {
    state
        .core_operations
        .install_and_start(Arc::new(TauriOperationEventSink { app }))
        .await
        .map_err(|error| error.to_string())
}

async fn check_core_update_for(state: &DesktopState) -> Result<CoreUpdateCheck, String> {
    state
        .core_operations
        .check_update()
        .await
        .map_err(|error| error.to_string())
}

async fn install_core_update_for(
    expected_version: String,
    app: tauri::AppHandle,
    state: &DesktopState,
) -> Result<CoreOperationSnapshot, String> {
    state
        .core_operations
        .install_update(&expected_version, Arc::new(TauriOperationEventSink { app }))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn system_locale() -> Option<String> {
    wokrouter_platform::detect_system_locale()
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

#[tauri::command]
async fn core_operation_status(
    state: tauri::State<'_, DesktopState>,
) -> Result<Option<CoreOperationSnapshot>, String> {
    core_operation_status_for(&state).await
}

#[tauri::command]
async fn install_and_start_core(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<CoreOperationSnapshot, String> {
    install_and_start_core_for(app, &state).await
}

#[tauri::command]
async fn check_core_update(
    state: tauri::State<'_, DesktopState>,
) -> Result<CoreUpdateCheck, String> {
    check_core_update_for(&state).await
}

#[tauri::command]
async fn install_core_update(
    expected_version: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<CoreOperationSnapshot, String> {
    install_core_update_for(expected_version, app, &state).await
}

pub fn run() -> tauri::Result<()> {
    let runtime = Arc::new(DesktopRuntimeState::discover());
    let state = DesktopState::new(
        DesktopControl::new(runtime.clone()),
        CoreOperationCoordinator::new(runtime.clone()),
    );
    tauri::Builder::default()
        .manage(state)
        .manage(ManagementState::discover(runtime))
        .invoke_handler(tauri::generate_handler![
            system_locale,
            core_status,
            start_core,
            stop_core,
            core_operation_status,
            install_and_start_core,
            check_core_update,
            install_core_update,
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
    use super::*;

    #[test]
    fn system_locale_command_returns_only_a_safe_candidate() {
        if let Some(locale) = system_locale() {
            assert!(!locale.is_empty());
            assert!(!locale.contains(['/', '\\']));
            assert!(!locale.chars().any(char::is_control));
        }
    }
}
