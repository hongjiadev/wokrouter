use std::fs;

use tempfile::tempdir;
use wokrouter_platform::{PlatformError, discover_wokcore_executable};

#[cfg(any(not(windows), feature = "test-support"))]
#[test]
fn verified_install_record_precedes_path_discovery() {
    let fixture = tempdir().unwrap();
    let executable = fixture
        .path()
        .join(format!("wokcore{}", std::env::consts::EXE_SUFFIX));
    fs::write(&executable, b"synthetic executable").unwrap();
    make_executable(&executable);
    let record = fixture.path().join("wokcore-install.json");
    fs::write(
        &record,
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "executable": executable
        }))
        .unwrap(),
    )
    .unwrap();
    secure_record(&record);

    assert_eq!(
        discover_wokcore_executable(&record).unwrap(),
        Some(executable)
    );
}

#[test]
fn invalid_install_record_fails_closed_without_exposing_its_path() {
    let fixture = tempdir().unwrap();
    let record = fixture.path().join("wokcore-install.json");
    fs::write(
        &record,
        br#"{"schema_version":1,"executable":"relative/wokcore"}"#,
    )
    .unwrap();
    secure_record(&record);

    let error = discover_wokcore_executable(&record).unwrap_err();

    assert!(matches!(error, PlatformError::InvalidWokCoreInstallRecord));
    assert!(!error.to_string().contains("relative"));
    assert!(!format!("{error:?}").contains("relative"));
}

#[cfg(unix)]
fn make_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

#[cfg(all(windows, feature = "test-support"))]
fn make_executable(path: &std::path::Path) {
    wokrouter_platform::test_support::secure_private_file(path).unwrap();
}

#[cfg(not(any(unix, windows)))]
fn make_executable(_path: &std::path::Path) {}

#[cfg(unix)]
fn secure_record(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[cfg(all(windows, feature = "test-support"))]
fn secure_record(path: &std::path::Path) {
    wokrouter_platform::test_support::secure_private_file(path).unwrap();
}

#[cfg(not(any(unix, all(windows, feature = "test-support"))))]
fn secure_record(_path: &std::path::Path) {}
