use std::fs;

use tempfile::tempdir;
use wokrouter_platform::{
    ClientIntegrationManager, ClientRoots, DoctorStatus, IntegrationDoctor, IntegrationError,
};

#[test]
fn doctor_reports_native_clients_without_mutating_fake_home() {
    let fixture = tempdir().unwrap();
    let home = fixture.path().join("home");
    let roots = ClientRoots {
        home: home.clone(),
        codex_config: home.join(".codex").join("config.toml"),
        claude_settings: home.join(".claude").join("settings.json"),
        copilot_data: home.join(".copilot-app"),
    };
    fs::create_dir_all(roots.codex_config.parent().unwrap()).unwrap();
    fs::create_dir_all(roots.claude_settings.parent().unwrap()).unwrap();
    fs::write(&roots.codex_config, "# native\nmodel = \"native\"\n").unwrap();
    fs::write(&roots.claude_settings, b"{\"theme\":\"dark\"}\n").unwrap();
    let state = fixture.path().join("state");
    let manager = ClientIntegrationManager::open_read_only(roots.clone(), state.clone()).unwrap();
    let before_codex = fs::read(&roots.codex_config).unwrap();
    let before_claude = fs::read(&roots.claude_settings).unwrap();

    let report = IntegrationDoctor::inspect(&manager).unwrap();

    assert_eq!(
        report
            .checks
            .iter()
            .find(|check| check.id == "codex_config")
            .unwrap()
            .status,
        DoctorStatus::Missing
    );
    assert_eq!(
        report
            .checks
            .iter()
            .find(|check| check.id == "claude_config")
            .unwrap()
            .status,
        DoctorStatus::Missing
    );
    assert_eq!(fs::read(&roots.codex_config).unwrap(), before_codex);
    assert_eq!(fs::read(&roots.claude_settings).unwrap(), before_claude);
    assert!(!state.exists());
    let rendered = serde_json::to_string(&report).unwrap();
    assert!(!rendered.contains("native-model"));
    assert!(!rendered.contains(fixture.path().to_string_lossy().as_ref()));
}

#[test]
fn doctor_reports_invalid_client_config_without_aborting_or_rewriting_it() {
    let fixture = tempdir().unwrap();
    let home = fixture.path().join("home");
    let codex = home.join(".codex").join("config.toml");
    fs::create_dir_all(codex.parent().unwrap()).unwrap();
    fs::write(&codex, b"[invalid").unwrap();
    let roots = ClientRoots {
        home: home.clone(),
        codex_config: codex.clone(),
        claude_settings: home.join(".claude").join("settings.json"),
        copilot_data: home.join(".copilot-app"),
    };
    let manager =
        ClientIntegrationManager::open_read_only(roots, fixture.path().join("state")).unwrap();

    let report = IntegrationDoctor::inspect(&manager).unwrap();

    let check = report
        .checks
        .iter()
        .find(|check| check.id == "codex_config")
        .unwrap();
    assert_eq!(check.status, DoctorStatus::Conflict);
    assert_eq!(check.summary_key, "integration.codex.invalid_config");
    assert_eq!(fs::read(&codex).unwrap(), b"[invalid");
}

#[test]
fn read_only_manager_rejects_client_config_roots_outside_the_fake_home() {
    let fixture = tempdir().unwrap();
    let home = fixture.path().join("home");
    fs::create_dir(&home).unwrap();
    let roots = ClientRoots {
        home: home.clone(),
        codex_config: fixture.path().join("real-config.toml"),
        claude_settings: home.join(".claude").join("settings.json"),
        copilot_data: home.join(".copilot-app"),
    };

    assert_eq!(
        ClientIntegrationManager::open_read_only(roots, fixture.path().join("state")).unwrap_err(),
        IntegrationError::InvalidState
    );
}

#[test]
fn doctor_rejects_a_symlinked_client_directory_without_following_it() {
    let fixture = tempdir().unwrap();
    let home = fixture.path().join("home");
    let outside = fixture.path().join("outside");
    fs::create_dir(&home).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("config.toml"), b"model = \"outside\"\n").unwrap();
    if symlink_directory(&outside, &home.join(".codex")).is_err() {
        return;
    }
    let roots = ClientRoots {
        home: home.clone(),
        codex_config: home.join(".codex").join("config.toml"),
        claude_settings: home.join(".claude").join("settings.json"),
        copilot_data: home.join(".copilot-app"),
    };
    let manager =
        ClientIntegrationManager::open_read_only(roots, fixture.path().join("state")).unwrap();

    let report = IntegrationDoctor::inspect(&manager).unwrap();

    assert_eq!(
        report
            .checks
            .iter()
            .find(|check| check.id == "codex_config")
            .unwrap()
            .status,
        DoctorStatus::Conflict
    );
}

#[cfg(unix)]
fn symlink_directory(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink_directory(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

#[cfg(not(any(unix, windows)))]
fn symlink_directory(_target: &std::path::Path, _link: &std::path::Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "symlinks are unsupported",
    ))
}
