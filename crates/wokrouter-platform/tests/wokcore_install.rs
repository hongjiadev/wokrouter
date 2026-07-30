#![cfg(feature = "test-support")]

use std::{fs, path::Path, sync::mpsc};

use semver::Version;
use sha2::{Digest, Sha256};
use tempfile::tempdir;
use url::Url;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};
use wokrouter_platform::{
    AppPaths, WokCoreInstallError, WokCoreInstallOutcome, WokCoreInstallPhase,
    WokCoreInstallProgress, WokCoreInstallProgressObserver, WokCoreInstallSource,
    install_missing_wokcore, install_missing_wokcore_with_progress,
};

const PUBLIC_KEY: &str = include_str!("fixtures/wokcore-install/minisign.pub");
const MANIFEST: &[u8] = include_bytes!("fixtures/wokcore-install/wokcore-update-v1.json");
const SIGNATURE: &[u8] = include_bytes!("fixtures/wokcore-install/wokcore-update-v1.json.minisig");
const V2_MANIFEST: &[u8] = include_bytes!("fixtures/wokcore-install/wokcore-update-v2.json");
const V2_SIGNATURE: &[u8] =
    include_bytes!("fixtures/wokcore-install/wokcore-update-v2.json.minisig");
const WINDOWS_ARCHIVE: &[u8] = &[
    80, 75, 3, 4, 20, 0, 0, 0, 0, 0, 0, 0, 33, 0, 248, 159, 107, 102, 14, 0, 0, 0, 14, 0, 0, 0, 11,
    0, 0, 0, 119, 111, 107, 99, 111, 114, 101, 46, 101, 120, 101, 110, 101, 119, 32, 101, 120, 101,
    99, 117, 116, 97, 98, 108, 101, 80, 75, 1, 2, 20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 33, 0, 248, 159,
    107, 102, 14, 0, 0, 0, 14, 0, 0, 0, 11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 128, 1, 0, 0, 0, 0,
    119, 111, 107, 99, 111, 114, 101, 46, 101, 120, 101, 80, 75, 5, 6, 0, 0, 0, 0, 1, 0, 1, 0, 57,
    0, 0, 0, 55, 0, 0, 0, 0, 0,
];
const UNIX_ARCHIVE: &[u8] = &[
    31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 237, 205, 65, 10, 130, 80, 20, 5, 208, 183, 20, 151, 240,
    165, 204, 245, 152, 188, 81, 145, 96, 138, 45, 191, 143, 147, 160, 121, 65, 116, 206, 228, 94,
    238, 228, 110, 211, 101, 156, 230, 140, 79, 42, 85, 223, 117, 123, 86, 239, 89, 74, 123, 122,
    245, 125, 239, 15, 199, 54, 154, 18, 95, 176, 222, 151, 97, 174, 151, 241, 159, 110, 185, 53,
    249, 200, 113, 93, 134, 243, 53, 3, 0, 0, 0, 0, 0, 0, 0, 0, 128, 31, 241, 4, 159, 198, 218, 25,
    0, 40, 0, 0,
];

#[cfg(windows)]
const ARCHIVE: &[u8] = WINDOWS_ARCHIVE;
#[cfg(not(windows))]
const ARCHIVE: &[u8] = UNIX_ARCHIVE;

#[derive(Default)]
struct RecordingProgress(Vec<WokCoreInstallProgress>);

impl WokCoreInstallProgressObserver for RecordingProgress {
    fn on_progress(&mut self, event: WokCoreInstallProgress) {
        self.0.push(event);
    }
}

struct DisconnectedProgress(mpsc::Sender<WokCoreInstallProgress>);

impl WokCoreInstallProgressObserver for DisconnectedProgress {
    fn on_progress(&mut self, event: WokCoreInstallProgress) {
        let _ = self.0.send(event);
    }
}

#[test]
fn archive_fixtures_do_not_drift_from_signed_manifest() {
    assert_eq!(MANIFEST.len(), 1701);
    assert_eq!(
        format!("{:x}", Sha256::digest(MANIFEST)),
        "eaee3c283f5ed4c797aeaab8740220a607757ada4c2fb8c47887201947973a4c"
    );
    assert_eq!(SIGNATURE.len(), 293);
    assert_eq!(
        format!("{:x}", Sha256::digest(SIGNATURE)),
        "bec7a76ca9e2acc62062bd3bb0cca006365e2fda7453a56bada3c3cb6ff56a59"
    );
    assert_eq!(V2_MANIFEST.len(), 1934);
    assert_eq!(
        format!("{:x}", Sha256::digest(V2_MANIFEST)),
        "2cacdcbe85345250dacc649bf019ce855e25b6c11aa70eff4ad4c75a90ba385b"
    );
    assert_eq!(V2_SIGNATURE.len(), 293);
    assert_eq!(
        format!("{:x}", Sha256::digest(V2_SIGNATURE)),
        "36e9c83a7b8f7e71997a91c0184da21f17af5749d77e3a773c1df5203fb96afd"
    );
    assert_eq!(PUBLIC_KEY.len(), 113);
    assert_eq!(
        format!("{:x}", Sha256::digest(PUBLIC_KEY.as_bytes())),
        "1dc2696979ab17a3c92c6934f9120c5e3a456fbe67c28df68a7cb3ee28586a61"
    );
    assert_eq!(WINDOWS_ARCHIVE.len(), 134);
    assert_eq!(
        format!("{:x}", Sha256::digest(WINDOWS_ARCHIVE)),
        "8af7e44ead86be8d0f7db9e445384231287891e1eee2873e538519ee0af2d06b"
    );
    assert_eq!(UNIX_ARCHIVE.len(), 111);
    assert_eq!(
        format!("{:x}", Sha256::digest(UNIX_ARCHIVE)),
        "ea04aa3f9d33dcdafbc06322ef82d35a385e263174443a3d00603318be1e4db4"
    );
}

#[test]
fn production_source_is_pinned_and_test_sources_are_ipv4_loopback_only() {
    let production = WokCoreInstallSource::production().unwrap();
    assert_eq!(
        production.origin().as_str(),
        "https://github.com/hongjiadev/wokcore/releases/latest/download/"
    );
    assert_eq!(production.public_key_id(), "7EF262CD8E9FE136");

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
async fn signed_release_reports_monotonic_download_and_authoritative_install_phases() {
    let server = signed_release_server(ARCHIVE).await;
    let fixture = tempdir().unwrap();
    let paths = app_paths(fixture.path());
    let source = WokCoreInstallSource::loopback(
        Url::parse(&format!("{}/releases/", server.uri())).unwrap(),
        PUBLIC_KEY,
    )
    .unwrap();
    let mut progress = RecordingProgress::default();

    let outcome = install_missing_wokcore_with_progress(&paths, &source, &mut progress)
        .await
        .unwrap();

    assert!(matches!(
        outcome,
        WokCoreInstallOutcome::Installed {
            version,
            ..
        } if version == Version::new(1, 2, 3)
    ));
    assert_eq!(
        progress.0.first(),
        Some(&WokCoreInstallProgress {
            phase: WokCoreInstallPhase::CheckingRelease,
            target_version: None,
            bytes_completed: None,
            bytes_total: None,
        })
    );
    let downloads = progress
        .0
        .iter()
        .filter(|event| event.phase == WokCoreInstallPhase::Downloading)
        .collect::<Vec<_>>();
    assert_eq!(
        downloads.first().and_then(|event| event.bytes_completed),
        Some(0)
    );
    assert_eq!(
        downloads.last().and_then(|event| event.bytes_completed),
        Some(ARCHIVE.len() as u64)
    );
    assert!(downloads.iter().all(|event| {
        event.target_version == Some(Version::new(1, 2, 3))
            && event.bytes_total == Some(ARCHIVE.len() as u64)
            && event.bytes_completed <= event.bytes_total
    }));
    assert!(
        downloads
            .windows(2)
            .all(|pair| pair[0].bytes_completed <= pair[1].bytes_completed)
    );
    let verifying = WokCoreInstallProgress {
        phase: WokCoreInstallPhase::Verifying,
        target_version: Some(Version::new(1, 2, 3)),
        bytes_completed: None,
        bytes_total: None,
    };
    let installing = WokCoreInstallProgress {
        phase: WokCoreInstallPhase::Installing,
        target_version: Some(Version::new(1, 2, 3)),
        bytes_completed: None,
        bytes_total: None,
    };
    let verifying_index = progress
        .0
        .iter()
        .position(|event| event == &verifying)
        .unwrap();
    let installing_index = progress
        .0
        .iter()
        .position(|event| event == &installing)
        .unwrap();
    assert!(verifying_index < installing_index);
    assert_eq!(installing_index, progress.0.len() - 1);
}

#[tokio::test]
async fn a_disconnected_progress_receiver_does_not_change_the_install_result() {
    let server = signed_release_server(ARCHIVE).await;
    let fixture = tempdir().unwrap();
    let paths = app_paths(fixture.path());
    let source = WokCoreInstallSource::loopback(
        Url::parse(&format!("{}/releases/", server.uri())).unwrap(),
        PUBLIC_KEY,
    )
    .unwrap();
    let (sender, receiver) = mpsc::channel();
    drop(receiver);
    let mut observer = DisconnectedProgress(sender);

    let outcome = install_missing_wokcore_with_progress(&paths, &source, &mut observer)
        .await
        .unwrap();

    assert!(matches!(outcome, WokCoreInstallOutcome::Installed { .. }));
}

#[tokio::test]
async fn wokcore_install_prefers_a_valid_signed_v2_release_without_requesting_v1() {
    let server = signed_v2_release_server(ARCHIVE).await;
    let fixture = tempdir().unwrap();
    let paths = app_paths(fixture.path());
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
}

#[tokio::test]
async fn wokcore_install_missing_v2_manifest_falls_back_to_the_signed_v1_release() {
    let server = signed_release_server(ARCHIVE).await;
    Mock::given(method("GET"))
        .and(path("/releases/wokcore-update-v2.json"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    let fixture = tempdir().unwrap();
    let paths = app_paths(fixture.path());
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
}

#[tokio::test]
async fn wokcore_install_present_invalid_v2_manifest_never_downgrades_to_v1() {
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
        ("/releases/wokcore-update-v2.json", V2_MANIFEST),
        (
            "/releases/wokcore-update-v2.json.minisig",
            corrupt_signature.as_slice(),
        ),
    ] {
        Mock::given(method("GET"))
            .and(wiremock::matchers::path(path))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
            .expect(1)
            .mount(&server)
            .await;
    }
    for path in [
        "/releases/wokcore-update-v1.json",
        "/releases/wokcore-update-v1.json.minisig",
    ] {
        Mock::given(method("GET"))
            .and(wiremock::matchers::path(path))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
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
async fn wokcore_install_rejects_a_signed_v1_schema_at_the_v2_endpoint_without_downgrading() {
    let server = MockServer::start().await;
    for (path, body) in [
        ("/releases/wokcore-update-v2.json", MANIFEST),
        ("/releases/wokcore-update-v2.json.minisig", SIGNATURE),
    ] {
        Mock::given(method("GET"))
            .and(wiremock::matchers::path(path))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
            .expect(1)
            .mount(&server)
            .await;
    }
    for path in [
        "/releases/wokcore-update-v1.json",
        "/releases/wokcore-update-v1.json.minisig",
    ] {
        Mock::given(method("GET"))
            .and(wiremock::matchers::path(path))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
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

    assert_eq!(error, WokCoreInstallError::IncompatibleManifest);
    assert!(!paths.wokcore_install_record.exists());
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

    let mut progress = RecordingProgress::default();
    let outcome = install_missing_wokcore_with_progress(&paths, &unreachable, &mut progress)
        .await
        .unwrap();

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
    assert!(progress.0.is_empty());
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

    let mut progress = RecordingProgress::default();
    let error = install_missing_wokcore_with_progress(&paths, &source, &mut progress)
        .await
        .unwrap_err();

    assert_eq!(error, WokCoreInstallError::ArtifactHashMismatch);
    assert_no_installing(&progress);
    assert!(!paths.wokcore_install_record.exists());
    assert!(
        !paths
            .wokcore_install_dir
            .join(format!("wokcore{}", std::env::consts::EXE_SUFFIX))
            .exists()
    );
}

#[tokio::test]
async fn artifact_size_mismatch_never_reports_installing() {
    let server = signed_release_server(&ARCHIVE[..ARCHIVE.len() - 1]).await;
    let fixture = tempdir().unwrap();
    let paths = app_paths(fixture.path());
    let source = WokCoreInstallSource::loopback(
        Url::parse(&format!("{}/releases/", server.uri())).unwrap(),
        PUBLIC_KEY,
    )
    .unwrap();
    let mut progress = RecordingProgress::default();

    let error = install_missing_wokcore_with_progress(&paths, &source, &mut progress)
        .await
        .unwrap_err();

    assert_eq!(error, WokCoreInstallError::ArtifactSizeMismatch);
    assert_no_installing(&progress);
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

    let mut progress = RecordingProgress::default();
    let error = install_missing_wokcore_with_progress(&paths, &source, &mut progress)
        .await
        .unwrap_err();

    assert_eq!(error, WokCoreInstallError::InvalidSignature);
    assert!(!paths.wokcore_install_record.exists());
    assert_no_installing(&progress);
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

async fn signed_v2_release_server(archive: &[u8]) -> MockServer {
    let server = MockServer::start().await;
    for (path, body) in [
        ("/releases/wokcore-update-v2.json", V2_MANIFEST),
        ("/releases/wokcore-update-v2.json.minisig", V2_SIGNATURE),
    ] {
        Mock::given(method("GET"))
            .and(wiremock::matchers::path(path))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
            .expect(1)
            .mount(&server)
            .await;
    }
    for path in [
        "/releases/wokcore-update-v1.json",
        "/releases/wokcore-update-v1.json.minisig",
    ] {
        Mock::given(method("GET"))
            .and(wiremock::matchers::path(path))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
    }
    Mock::given(method("GET"))
        .and(wiremock::matchers::path(format!(
            "/releases/{}",
            v2_artifact_name()
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(archive))
        .expect(1)
        .mount(&server)
        .await;
    server
}

fn v2_artifact_name() -> &'static str {
    if cfg!(all(target_arch = "x86_64", target_os = "windows")) {
        "WokCore-v1.2.3-Windows-x86_64-Portable.zip"
    } else if cfg!(all(target_arch = "aarch64", target_os = "windows")) {
        "WokCore-v1.2.3-Windows-arm64-Portable.zip"
    } else if cfg!(all(target_arch = "x86_64", target_os = "macos")) {
        "WokCore-v1.2.3-macOS-x86_64.tar.gz"
    } else if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
        "WokCore-v1.2.3-macOS-arm64.tar.gz"
    } else if cfg!(all(target_arch = "x86_64", target_os = "linux")) {
        "WokCore-v1.2.3-Linux-x86_64.tar.gz"
    } else if cfg!(all(target_arch = "aarch64", target_os = "linux")) {
        "WokCore-v1.2.3-Linux-arm64.tar.gz"
    } else {
        panic!("unsupported test target")
    }
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

fn assert_no_installing(progress: &RecordingProgress) {
    assert!(
        progress
            .0
            .iter()
            .all(|event| event.phase != WokCoreInstallPhase::Installing)
    );
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
