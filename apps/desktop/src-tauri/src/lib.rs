//! WokRouter desktop shell.

mod control;
mod core_operation;
mod runtime;
mod wokcore;

use std::sync::Arc;

#[cfg(feature = "packaged-acceptance")]
use std::{
    collections::BTreeSet,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

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

#[cfg(not(feature = "packaged-acceptance"))]
async fn core_status_for(state: &DesktopState) -> Result<CoreStatus, String> {
    state
        .control
        .status()
        .await
        .map_err(|error| error.to_string())
}

#[cfg(feature = "packaged-acceptance")]
async fn core_status_for(state: &DesktopState) -> Result<CoreStatus, String> {
    let _ = &state.control;
    packaged_acceptance_retain_production_control_contract();
    packaged_acceptance_core_status(std::env::var_os("WOKROUTER_ACCEPTANCE_STATE_ROOT").as_deref())
}

#[cfg(not(feature = "packaged-acceptance"))]
async fn start_core_for(state: &DesktopState) -> Result<(), String> {
    state
        .control
        .start()
        .await
        .map_err(|error| error.to_string())
}

#[cfg(feature = "packaged-acceptance")]
async fn start_core_for(state: &DesktopState) -> Result<(), String> {
    let _ = &state.control;
    packaged_acceptance_lifecycle_rejection()
}

#[cfg(not(feature = "packaged-acceptance"))]
async fn stop_core_for(state: &DesktopState) -> Result<(), String> {
    state
        .control
        .stop()
        .await
        .map_err(|error| error.to_string())
}

#[cfg(feature = "packaged-acceptance")]
async fn stop_core_for(state: &DesktopState) -> Result<(), String> {
    let _ = &state.control;
    packaged_acceptance_lifecycle_rejection()
}

async fn core_operation_status_for(
    state: &DesktopState,
    sink: Arc<dyn OperationEventSink>,
) -> Result<Option<CoreOperationSnapshot>, String> {
    state
        .core_operations
        .status_with_sink(sink)
        .await
        .map_err(|error| error.to_string())
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
    #[cfg(not(feature = "packaged-acceptance"))]
    {
        wokrouter_platform::detect_system_locale()
    }
    #[cfg(feature = "packaged-acceptance")]
    {
        packaged_acceptance_locale(
            std::env::var_os("WOKROUTER_PACKAGED_ACCEPTANCE_LOCALE").as_deref(),
        )
    }
}

#[cfg(feature = "packaged-acceptance")]
fn packaged_acceptance_locale(value: Option<&OsStr>) -> Option<String> {
    match value.and_then(OsStr::to_str) {
        Some("en-US") => Some("en-US".to_owned()),
        Some("zh-CN") => Some("zh-CN".to_owned()),
        Some("zh-TW") => Some("zh-TW".to_owned()),
        Some("none") | None => None,
        Some(_) => None,
    }
}

#[cfg(feature = "packaged-acceptance")]
fn packaged_acceptance_lifecycle_rejection() -> Result<(), String> {
    Err("acceptance_fail_closed".to_owned())
}

#[cfg(feature = "packaged-acceptance")]
fn packaged_acceptance_retain_production_control_contract() {
    let _ = DesktopControl::status;
    let _ = DesktopControl::start;
    let _ = DesktopControl::stop;
}

#[cfg(feature = "packaged-acceptance")]
fn packaged_acceptance_core_status(state_root: Option<&OsStr>) -> Result<CoreStatus, String> {
    let state_root = packaged_acceptance_state_root(state_root)?;
    let ready_path = state_root.join("serve-ready");
    if !ready_path.exists() {
        return Ok(CoreStatus::missing(
            wokrouter_platform::WokCoreRuntimeChannel::Production,
        ));
    }
    packaged_acceptance_regular_file(&ready_path, 64)?;

    let version_path = state_root.join("current-version.txt");
    let version = if version_path.exists() {
        packaged_acceptance_regular_file(&version_path, 64)?;
        fs::read_to_string(&version_path)
            .map_err(|_| "acceptance_state_unreadable".to_owned())?
            .trim()
            .to_owned()
    } else {
        "1.0.0".to_owned()
    };
    semver::Version::parse(&version).map_err(|_| "acceptance_version_invalid".to_owned())?;

    Ok(CoreStatus {
        state: wokrouter_cli::commands::CoreUiState::Running,
        runtime_channel: wokrouter_platform::WokCoreRuntimeChannel::Production,
        version: Some(version),
        management_api_major: Some(1),
        capabilities: BTreeSet::from(["core.update.v1".to_owned()]),
        phase: Some(wokrouter_wokcore_client::ServicePhase::Running),
        active_requests: Some(0),
        error_code: None,
    })
}

#[cfg(feature = "packaged-acceptance")]
fn packaged_acceptance_state_root(value: Option<&OsStr>) -> Result<PathBuf, String> {
    let candidate = value
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "acceptance_state_root_missing".to_owned())?;
    packaged_acceptance_plain_directory(&candidate)?;
    let canonical =
        fs::canonicalize(&candidate).map_err(|_| "acceptance_state_root_invalid".to_owned())?;
    if canonical.file_name().and_then(OsStr::to_str) != Some("fixture-state") {
        return Err("acceptance_state_root_invalid".to_owned());
    }
    let scratch = canonical
        .parent()
        .ok_or_else(|| "acceptance_state_root_invalid".to_owned())?;
    packaged_acceptance_plain_directory(scratch)?;
    let scratch_leaf = scratch
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| "acceptance_state_root_invalid".to_owned())?;
    let Some(run_id) = scratch_leaf.strip_prefix("wokrouter-packaged-gui-live-") else {
        return Err("acceptance_state_root_invalid".to_owned());
    };
    if run_id.len() != 32 || !run_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("acceptance_state_root_invalid".to_owned());
    }
    let temporary = fs::canonicalize(std::env::temp_dir())
        .map_err(|_| "acceptance_state_root_invalid".to_owned())?;
    if scratch.parent() != Some(temporary.as_path()) {
        return Err("acceptance_state_root_invalid".to_owned());
    }
    Ok(canonical)
}

#[cfg(feature = "packaged-acceptance")]
fn packaged_acceptance_plain_directory(path: &Path) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| "acceptance_state_root_invalid".to_owned())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("acceptance_state_root_invalid".to_owned());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("acceptance_state_root_invalid".to_owned());
        }
    }
    Ok(())
}

#[cfg(feature = "packaged-acceptance")]
fn packaged_acceptance_regular_file(path: &Path, maximum_bytes: u64) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| "acceptance_state_unreadable".to_owned())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > maximum_bytes {
        return Err("acceptance_state_unreadable".to_owned());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("acceptance_state_unreadable".to_owned());
        }
    }
    Ok(())
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
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<Option<CoreOperationSnapshot>, String> {
    core_operation_status_for(&state, Arc::new(TauriOperationEventSink { app })).await
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

pub fn run_core_operation_helper_if_requested() -> Option<u8> {
    core_operation::run_operation_helper_if_requested()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "packaged-acceptance")]
    struct AcceptanceStateRoot {
        scratch: std::path::PathBuf,
    }

    #[cfg(feature = "packaged-acceptance")]
    impl AcceptanceStateRoot {
        fn state(&self) -> std::path::PathBuf {
            self.scratch.join("fixture-state")
        }
    }

    #[cfg(feature = "packaged-acceptance")]
    impl Drop for AcceptanceStateRoot {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.scratch).expect("remove acceptance scratch root");
        }
    }

    #[cfg(feature = "packaged-acceptance")]
    fn acceptance_state_root() -> AcceptanceStateRoot {
        let scratch = std::env::temp_dir().join(format!(
            "wokrouter-packaged-gui-live-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&scratch).expect("scratch root");
        std::fs::create_dir(scratch.join("fixture-state")).expect("fixture state root");
        AcceptanceStateRoot { scratch }
    }

    #[test]
    fn system_locale_command_returns_only_a_safe_candidate() {
        if let Some(locale) = system_locale() {
            assert!(!locale.is_empty());
            assert!(!locale.contains(['/', '\\']));
            assert!(!locale.chars().any(char::is_control));
        }
    }

    #[cfg(feature = "packaged-acceptance")]
    #[test]
    fn packaged_acceptance_locale_is_an_exact_fail_closed_allowlist() {
        use std::ffi::OsStr;

        assert_eq!(
            packaged_acceptance_locale(Some(OsStr::new("en-US"))),
            Some("en-US".to_owned())
        );
        assert_eq!(
            packaged_acceptance_locale(Some(OsStr::new("zh-CN"))),
            Some("zh-CN".to_owned())
        );
        assert_eq!(
            packaged_acceptance_locale(Some(OsStr::new("zh-TW"))),
            Some("zh-TW".to_owned())
        );
        assert_eq!(packaged_acceptance_locale(Some(OsStr::new("none"))), None);
        assert_eq!(packaged_acceptance_locale(Some(OsStr::new("fr-FR"))), None);
        assert_eq!(packaged_acceptance_locale(None), None);
    }

    #[cfg(feature = "packaged-acceptance")]
    #[test]
    fn packaged_acceptance_status_moves_from_missing_to_running() {
        let temporary = acceptance_state_root();
        let state = temporary.state();

        let missing = packaged_acceptance_core_status(Some(state.as_os_str()))
            .expect("missing acceptance status");
        assert_eq!(missing.state, wokrouter_cli::commands::CoreUiState::Missing);
        assert_eq!(
            missing.runtime_channel,
            wokrouter_platform::WokCoreRuntimeChannel::Production
        );

        std::fs::write(state.join("current-version.txt"), "2.3.4\n").expect("fixture version");
        std::fs::write(state.join("serve-ready"), "ready\n").expect("serve ready");
        let running = packaged_acceptance_core_status(Some(state.as_os_str()))
            .expect("running acceptance status");
        assert_eq!(running.state, wokrouter_cli::commands::CoreUiState::Running);
        assert_eq!(running.version.as_deref(), Some("2.3.4"));
        assert_eq!(running.management_api_major, Some(1));
        assert!(running.capabilities.contains("core.update.v1"));
        assert_eq!(
            running.phase,
            Some(wokrouter_wokcore_client::ServicePhase::Running)
        );
        assert_eq!(running.active_requests, Some(0));
        assert_eq!(running.error_code, None);
    }

    #[cfg(feature = "packaged-acceptance")]
    #[test]
    fn packaged_acceptance_status_rejects_untrusted_roots_and_versions() {
        use std::ffi::OsStr;

        assert!(packaged_acceptance_core_status(None).is_err());
        assert!(
            packaged_acceptance_core_status(Some(OsStr::new("not-a-live-scratch-root"))).is_err()
        );

        let temporary = acceptance_state_root();
        let state = temporary.state();
        std::fs::write(state.join("current-version.txt"), "not-semver").expect("invalid version");
        std::fs::write(state.join("serve-ready"), "ready").expect("serve ready");
        assert!(packaged_acceptance_core_status(Some(state.as_os_str())).is_err());
    }

    #[cfg(feature = "packaged-acceptance")]
    #[test]
    fn packaged_acceptance_lifecycle_is_always_fail_closed() {
        assert_eq!(
            packaged_acceptance_lifecycle_rejection(),
            Err("acceptance_fail_closed".to_owned())
        );
    }
}
