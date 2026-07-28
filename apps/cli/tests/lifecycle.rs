use std::{
    collections::BTreeSet,
    process::{Command, Output, Stdio},
};

use wokrouter_cli::commands::{CoreStatus, CoreUiState};
use wokrouter_wokcore_client::ServicePhase;

#[test]
fn missing_wokcore_has_stable_status_start_and_stop_contracts() {
    let home = tempfile::tempdir().unwrap();
    let empty_path = home.path().join("empty-path");
    std::fs::create_dir(&empty_path).unwrap();

    let status = run(&home, &empty_path, &["status", "--json"]);
    assert_eq!(status.status.code(), Some(3));
    assert_eq!(
        String::from_utf8(status.stdout).unwrap(),
        "{\"state\":\"missing\",\"capabilities\":[],\"error_code\":\"missing\"}\n"
    );

    let start = run(&home, &empty_path, &["start"]);
    assert!(!start.status.success());
    assert_eq!(
        String::from_utf8(start.stderr).unwrap(),
        "wokrouter: WokCore is not installed or is not available on PATH\n"
    );

    let stop = run(&home, &empty_path, &["stop"]);
    assert!(stop.status.success());
    assert_eq!(
        String::from_utf8(stop.stdout).unwrap(),
        "WokCore is already stopped.\n"
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

fn run(home: &tempfile::TempDir, search_path: &std::path::Path, arguments: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_wokrouter"));
    command
        .args(arguments)
        .env("PATH", search_path)
        .env("APPDATA", home.path().join("config"))
        .env("LOCALAPPDATA", home.path().join("state"))
        .env("USERPROFILE", home.path())
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env_remove("XDG_RUNTIME_DIR")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}
