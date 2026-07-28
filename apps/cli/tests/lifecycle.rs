use std::collections::BTreeSet;

use wokrouter_cli::commands::{CommandError, CoreStatus, CoreUiState};
use wokrouter_platform::AppPaths;
use wokrouter_wokcore_client::ServicePhase;

#[tokio::test]
async fn missing_wokcore_has_stable_status_start_and_stop_contracts() {
    let home = tempfile::tempdir().unwrap();
    let paths = isolated_paths(&home);
    let (status, exit_code) = wokrouter_cli::commands::status::snapshot(&paths)
        .await
        .unwrap();

    assert_eq!(exit_code, 3);
    assert_eq!(status.state, CoreUiState::Missing);
    assert_eq!(
        serde_json::to_string(&status).unwrap(),
        "{\"state\":\"missing\",\"capabilities\":[],\"error_code\":\"missing\"}"
    );

    assert_eq!(
        wokrouter_cli::commands::start::execute(&paths)
            .await
            .unwrap_err(),
        CommandError::WokCoreMissing
    );
    assert_eq!(
        wokrouter_cli::commands::stop::execute(&paths)
            .await
            .unwrap(),
        0
    );
}

#[test]
fn running_status_dto_contains_no_token_or_executable_path() {
    let status = CoreStatus {
        state: CoreUiState::Running,
        version: Some("0.1.0".to_owned()),
        management_api_major: Some(1),
        capabilities: BTreeSet::from(["service.status".to_owned()]),
        phase: Some(ServicePhase::Running),
        active_requests: Some(7),
        error_code: None,
    };

    assert_eq!(
        serde_json::to_string(&status).unwrap(),
        "{\"state\":\"running\",\"version\":\"0.1.0\",\"management_api_major\":1,\"capabilities\":[\"service.status\"],\"phase\":\"running\",\"active_requests\":7}"
    );
}

fn isolated_paths(home: &tempfile::TempDir) -> AppPaths {
    AppPaths {
        config_file: home.path().join("config").join("config.toml"),
        wokcore_install_record: home.path().join("config").join("wokcore-install.json"),
        state_db: home.path().join("state").join("state.sqlite3"),
        runtime_dir: home.path().join("runtime"),
        log_dir: home.path().join("logs"),
        wokcore_discovery_file: home.path().join("runtime").join("discovery.json"),
    }
}
