#![cfg(feature = "test-support")]

use std::{fs, path::Path};

use semver::Version;
use tempfile::tempdir;
use url::Url;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};
use wokrouter_platform::{
    AppPaths, WokCoreInstallError, WokCoreInstallOutcome, WokCoreInstallSource,
    install_missing_wokcore,
};

const PUBLIC_KEY: &str = include_str!("fixtures/wokcore-install/minisign.pub");
const MANIFEST: &[u8] = include_bytes!("fixtures/wokcore-install/wokcore-update-v1.json");
const SIGNATURE: &[u8] = include_bytes!("fixtures/wokcore-install/wokcore-update-v1.json.minisig");
#[cfg(windows)]
const ARCHIVE: &[u8] = &[
    80, 75, 3, 4, 20, 0, 0, 0, 0, 0, 0, 0, 33, 0, 248, 159, 107, 102, 14, 0, 0, 0, 14, 0, 0, 0, 11,
    0, 0, 0, 119, 111, 107, 99, 111, 114, 101, 46, 101, 120, 101, 110, 101, 119, 32, 101, 120, 101,
    99, 117, 116, 97, 98, 108, 101, 80, 75, 1, 2, 20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 33, 0, 248, 159,
    107, 102, 14, 0, 0, 0, 14, 0, 0, 0, 11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 128, 1, 0, 0, 0, 0,
    119, 111, 107, 99, 111, 114, 101, 46, 101, 120, 101, 80, 75, 5, 6, 0, 0, 0, 0, 1, 0, 1, 0, 57,
    0, 0, 0, 55, 0, 0, 0, 0, 0,
];
#[cfg(not(windows))]
const ARCHIVE: &[u8] = &[
    31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 237, 205, 65, 10, 130, 80, 20, 5, 208, 183, 20, 151, 240,
    165, 204, 245, 152, 188, 81, 145, 96, 138, 45, 191, 143, 147, 160, 121, 65, 116, 206, 228, 94,
    238, 228, 110, 211, 101, 230, 140, 79, 42, 85, 223, 117, 123, 86, 239, 89, 74, 123, 122, 245,
    125, 239, 15, 199, 54, 154, 18, 95, 176, 222, 151, 97, 174, 151, 241, 159, 110, 185, 53, 249,
    200, 113, 93, 134, 243, 53, 3, 0, 0, 0, 0, 0, 0, 0, 0, 128, 31, 241, 4, 159, 198, 218, 25, 0,
    40, 0, 0,
];

#[test]
fn production_source_is_pinned_and_test_sources_are_ipv4_loopback_only() {
    let production = WokCoreInstallSource::production(PUBLIC_KEY).unwrap();
    assert_eq!(
        production.origin().as_str(),
        "https://github.com/hongjiadev/wokcore/releases/latest/download/"
    );

    assert!(
        WokCoreInstallSource::loopback(
            Url::parse("http://127.0.0.1:32123/releases/").unwrap(),
            PUBLIC_KEY,
        )
        .is_ok()
    );
    for rejected in [
        "http://localhost:32123/releases/",
        "http://[::1]:32123/releases/",
        "http://127.0.0.1:0/releases/",
        "http://user@127.0.0.1:32123/releases/",
        "http://127.0.0.1:32123/releases/?channel=test",
        "https://127.0.0.1:32123/releases/",
    ] {
        assert!(
            WokCoreInstallSource::loopback(Url::parse(rejected).unwrap(), PUBLIC_KEY).is_err(),
            "{rejected}"
        );
    }
}

#[tokio::test]
async fn signed_release_is_downloaded_and_atomically_registered() {
    let server = signed_release_server(ARCHIVE).await;
    let fixture = tempdir().unwrap();
    let paths = app_paths(fixture.path());
    let source = WokCoreInstallSource::loopback(
        Url::parse(&format!("{}/releases/", server.uri())).unwrap(),
        PUBLIC_KEY,
    )
    .unwrap();

    let outcome = install_missing_wokcore(&paths, &source).await.unwrap();

    let executable = paths
        .wokcore_install_dir
        .join(format!("wokcore{}", std::env::consts::EXE_SUFFIX));
    assert_eq!(
        outcome,
        WokCoreInstallOutcome::Installed {
            version: Version::new(1, 2, 3),
            executable: executable.clone(),
        }
    );
    assert_eq!(fs::read(&executable).unwrap(), b"new executable");
    assert!(paths.wokcore_install_record.is_file());
    assert_secure_install_permissions(&paths, &executable);
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(&paths.wokcore_install_record).unwrap()).unwrap();
    assert_eq!(
        record.get("executable").and_then(serde_json::Value::as_str),
        executable.to_str()
    );
    let entries = fs::read_dir(&paths.wokcore_install_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let mut entries = entries;
    entries.sort();
    assert_eq!(
        entries,
        vec![
            ".wokcore-install.lock".to_owned(),
            executable
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        ]
    );
}

#[tokio::test]
async fn an_existing_compatible_install_is_never_overwritten() {
    let server = signed_release_server(ARCHIVE).await;
    let fixture = tempdir().unwrap();
    let paths = app_paths(fixture.path());
    let source = WokCoreInstallSource::loopback(
        Url::parse(&format!("{}/releases/", server.uri())).unwrap(),
        PUBLIC_KEY,
    )
    .unwrap();
    let installed = install_missing_wokcore(&paths, &source).await.unwrap();
    let executable = match installed {
        WokCoreInstallOutcome::Installed { executable, .. } => executable,
        WokCoreInstallOutcome::AlreadyInstalled { .. } => panic!("expected a new install"),
    };
    fs::write(&executable, b"newer compatible executable").unwrap();
    make_executable(&executable);
    let unreachable = WokCoreInstallSource::loopback(
        Url::parse("http://127.0.0.1:9/releases/").unwrap(),
        PUBLIC_KEY,
    )
    .unwrap();

    let outcome = install_missing_wokcore(&paths, &unreachable).await.unwrap();

    assert_eq!(
        outcome,
        WokCoreInstallOutcome::AlreadyInstalled {
            executable: executable.clone(),
        }
    );
    assert_eq!(
        fs::read(executable).unwrap(),
        b"newer compatible executable"
    );
}

#[tokio::test]
async fn installing_wokcore_does_not_modify_wokrouter_binary_or_version() {
    let server = signed_release_server(ARCHIVE).await;
    let fixture = tempdir().unwrap();
    let paths = app_paths(fixture.path());
    let router_dir = fixture.path().join("WokRouter");
    fs::create_dir_all(&router_dir).unwrap();
    let router_binary = router_dir.join(format!("wokrouter{}", std::env::consts::EXE_SUFFIX));
    let router_version = router_dir.join("version");
    fs::write(&router_binary, b"wokrouter binary 9.8.7").unwrap();
    fs::write(&router_version, b"9.8.7").unwrap();
    let source = WokCoreInstallSource::loopback(
        Url::parse(&format!("{}/releases/", server.uri())).unwrap(),
        PUBLIC_KEY,
    )
    .unwrap();

    let outcome = install_missing_wokcore(&paths, &source).await.unwrap();

    assert!(matches!(
        outcome,
        WokCoreInstallOutcome::Installed {
            version,
            ..
        } if version == Version::new(1, 2, 3)
    ));
    assert_eq!(fs::read(router_binary).unwrap(), b"wokrouter binary 9.8.7");
    assert_eq!(fs::read(router_version).unwrap(), b"9.8.7");
}

#[tokio::test]
async fn artifact_hash_mismatch_leaves_no_install_or_record() {
    let mut corrupt = ARCHIVE.to_vec();
    corrupt[10] ^= 0x01;
    let server = signed_release_server(&corrupt).await;
    let fixture = tempdir().unwrap();
    let paths = app_paths(fixture.path());
    let source = WokCoreInstallSource::loopback(
        Url::parse(&format!("{}/releases/", server.uri())).unwrap(),
        PUBLIC_KEY,
    )
    .unwrap();

    let error = install_missing_wokcore(&paths, &source).await.unwrap_err();

    assert_eq!(error, WokCoreInstallError::ArtifactHashMismatch);
    assert!(!paths.wokcore_install_record.exists());
    assert!(
        !paths
            .wokcore_install_dir
            .join(format!("wokcore{}", std::env::consts::EXE_SUFFIX))
            .exists()
    );
}

#[tokio::test]
async fn invalid_manifest_signature_is_rejected_before_artifact_download() {
    let server = MockServer::start().await;
    let mut corrupt_signature = SIGNATURE.to_vec();
    let payload = corrupt_signature
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|index| index + 1)
        .unwrap();
    corrupt_signature[payload] = if corrupt_signature[payload] == b'A' {
        b'B'
    } else {
        b'A'
    };
    for (path, body) in [
        ("/releases/wokcore-update-v1.json", MANIFEST),
        (
            "/releases/wokcore-update-v1.json.minisig",
            corrupt_signature.as_slice(),
        ),
    ] {
        Mock::given(method("GET"))
            .and(wiremock::matchers::path(path))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
            .mount(&server)
            .await;
    }
    let fixture = tempdir().unwrap();
    let paths = app_paths(fixture.path());
    let source = WokCoreInstallSource::loopback(
        Url::parse(&format!("{}/releases/", server.uri())).unwrap(),
        PUBLIC_KEY,
    )
    .unwrap();

    let error = install_missing_wokcore(&paths, &source).await.unwrap_err();

    assert_eq!(error, WokCoreInstallError::InvalidSignature);
    assert!(!paths.wokcore_install_record.exists());
}

#[tokio::test]
async fn unsafe_install_directory_fails_before_network_access() {
    let fixture = tempdir().unwrap();
    let paths = app_paths(fixture.path());
    fs::create_dir_all(paths.wokcore_install_dir.parent().unwrap()).unwrap();
    fs::write(&paths.wokcore_install_dir, b"not a directory").unwrap();
    let source = WokCoreInstallSource::loopback(
        Url::parse("http://127.0.0.1:9/releases/").unwrap(),
        PUBLIC_KEY,
    )
    .unwrap();

    let error = install_missing_wokcore(&paths, &source).await.unwrap_err();

    assert_eq!(error, WokCoreInstallError::UnsafeInstallLocation);
}

#[tokio::test]
async fn redirects_and_response_bodies_are_rejected_without_leaking_details() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/releases/wokcore-update-v1.json"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("Location", "https://example.com/sensitive-token"),
        )
        .mount(&server)
        .await;
    let fixture = tempdir().unwrap();
    let paths = app_paths(fixture.path());
    let source = WokCoreInstallSource::loopback(
        Url::parse(&format!("{}/releases/", server.uri())).unwrap(),
        PUBLIC_KEY,
    )
    .unwrap();

    let error = install_missing_wokcore(&paths, &source).await.unwrap_err();
    let display = error.to_string();
    let debug = format!("{error:?}");

    assert_eq!(error, WokCoreInstallError::DownloadFailed);
    assert!(!display.contains("sensitive-token"));
    assert!(!debug.contains("sensitive-token"));
    assert!(!display.contains(fixture.path().to_str().unwrap()));
    assert!(!debug.contains(fixture.path().to_str().unwrap()));
}

async fn signed_release_server(archive: &[u8]) -> MockServer {
    let server = MockServer::start().await;
    for (path, body) in [
        ("/releases/wokcore-update-v1.json", MANIFEST),
        ("/releases/wokcore-update-v1.json.minisig", SIGNATURE),
    ] {
        Mock::given(method("GET"))
            .and(wiremock::matchers::path(path))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
            .mount(&server)
            .await;
    }
    Mock::given(method("GET"))
        .and(wiremock::matchers::path(format!(
            "/releases/wokcore-v1.2.3-{}.{}",
            current_target(),
            if cfg!(windows) { "zip" } else { "tar.gz" }
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(archive))
        .mount(&server)
        .await;
    server
}

fn current_target() -> &'static str {
    if cfg!(all(target_arch = "x86_64", target_os = "windows")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_arch = "x86_64", target_os = "macos")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_arch = "x86_64", target_os = "linux")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_arch = "aarch64", target_os = "linux")) {
        "aarch64-unknown-linux-gnu"
    } else {
        panic!("unsupported test target")
    }
}

fn app_paths(root: &Path) -> AppPaths {
    AppPaths {
        config_file: root.join("config").join("config.toml"),
        wokcore_install_record: root.join("config").join("wokcore-install.json"),
        wokcore_install_dir: root.join("WokCore").join("bin"),
        integration_dir: root.join("state").join("integrations"),
        runtime_dir: root.join("runtime"),
        log_dir: root.join("logs"),
        wokcore_discovery_file: root.join("WokCore").join("runtime").join("discovery.json"),
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

#[cfg(unix)]
fn assert_secure_install_permissions(paths: &AppPaths, executable: &Path) {
    use std::os::unix::fs::PermissionsExt;

    assert_eq!(
        fs::metadata(&paths.wokcore_install_dir)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(paths.wokcore_install_record.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&paths.wokcore_install_record)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(executable).unwrap().permissions().mode() & 0o777,
        0o700
    );
}

#[cfg(windows)]
fn assert_secure_install_permissions(paths: &AppPaths, executable: &Path) {
    use wokrouter_platform::test_support::{is_private_directory, is_private_file};

    assert!(is_private_directory(&paths.wokcore_install_dir));
    assert!(is_private_directory(
        paths.wokcore_install_record.parent().unwrap()
    ));
    assert!(is_private_file(&paths.wokcore_install_record));
    assert!(is_private_file(executable));
}

#[cfg(not(any(unix, windows)))]
fn assert_secure_install_permissions(_paths: &AppPaths, _executable: &Path) {}
