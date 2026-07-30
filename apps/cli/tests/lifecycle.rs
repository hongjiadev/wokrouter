use std::collections::BTreeSet;

use wokrouter_cli::commands::{CommandError, CoreStatus, CoreUiState};
use wokrouter_platform::{AppPaths, WokCoreRuntimeChannel, select_wokcore_runtime};
use wokrouter_wokcore_client::ServicePhase;

#[tokio::test]
async fn missing_wokcore_has_stable_status_start_and_stop_contracts() {
    let home = tempfile::tempdir().unwrap();
    let paths = isolated_paths(&home);
    let runtime = select_wokcore_runtime(&paths).await.unwrap();
    let (status, exit_code) = wokrouter_cli::commands::status::snapshot_selected(&runtime)
        .await
        .unwrap();

    assert_eq!(exit_code, 3);
    assert_eq!(status.state, CoreUiState::Missing);
    assert_eq!(
        serde_json::to_string(&status).unwrap(),
        "{\"state\":\"missing\",\"runtime_channel\":\"production\",\"capabilities\":[],\"error_code\":\"missing\"}"
    );

    assert_eq!(
        wokrouter_cli::commands::start::execute(&runtime)
            .await
            .unwrap_err(),
        CommandError::WokCoreMissing
    );
    assert_eq!(
        wokrouter_cli::commands::stop::execute(&runtime)
            .await
            .unwrap(),
        0
    );
}

#[test]
fn running_status_dto_exposes_only_the_development_channel_and_public_health() {
    let status = CoreStatus {
        state: CoreUiState::Running,
        runtime_channel: WokCoreRuntimeChannel::Development,
        version: Some("0.1.0".to_owned()),
        management_api_major: Some(1),
        capabilities: BTreeSet::from(["service.status".to_owned()]),
        phase: Some(ServicePhase::Running),
        active_requests: Some(7),
        error_code: None,
    };

    assert_eq!(
        serde_json::to_string(&status).unwrap(),
        "{\"state\":\"running\",\"runtime_channel\":\"development\",\"version\":\"0.1.0\",\"management_api_major\":1,\"capabilities\":[\"service.status\"],\"phase\":\"running\",\"active_requests\":7}"
    );
    assert_no_private_runtime_fields(&serde_json::to_value(status).unwrap());
}

#[test]
fn development_runtime_management_error_has_stable_structured_and_human_forms() {
    let error = CommandError::DevelopmentRuntimeManagedByIde;

    assert_eq!(
        serde_json::to_string(&error).unwrap(),
        "\"development_runtime_managed_by_ide\""
    );
    assert_eq!(
        error.to_string(),
        "the development WokCore runtime is managed by the IDE"
    );
}

fn assert_no_private_runtime_fields(value: &serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                assert!(!matches!(key.as_str(), "pid" | "path" | "executable"));
                assert_no_private_runtime_fields(value);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                assert_no_private_runtime_fields(value);
            }
        }
        _ => {}
    }
}

fn isolated_paths(home: &tempfile::TempDir) -> AppPaths {
    AppPaths {
        config_file: home.path().join("config").join("config.toml"),
        wokcore_install_record: home.path().join("config").join("wokcore-install.json"),
        wokcore_install_dir: home.path().join("WokCore").join("bin"),
        integration_dir: home.path().join("state").join("integrations"),
        runtime_dir: home.path().join("runtime"),
        log_dir: home.path().join("logs"),
        wokcore_discovery_file: home.path().join("runtime").join("discovery.json"),
    }
}
