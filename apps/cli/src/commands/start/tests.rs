use std::{
    collections::VecDeque,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use tempfile::TempDir;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};
use wokrouter_platform::{
    AppPaths, PlatformError, WokCoreInstallError, WokCoreInstallSource, WokCoreRuntimeChannel,
};
use wokrouter_wokcore_client::{
    CoreConnection, ServiceError, ServicePhase, ServiceStatus, WokCoreClient,
};

use super::{
    StartCommandOutput, StartDependencies, StartOptions, StartService, StartedCore,
    execute_with_dependencies, install_error_code, render_structured_platform_error, spawn_command,
};
use crate::commands::{CommandError, CommandRuntime};

const PUBLIC_KEY: &str = include_str!(
    "../../../../../crates/wokrouter-platform/tests/fixtures/wokcore-install/minisign.pub"
);
const V2_MANIFEST: &[u8] = include_bytes!(
    "../../../../../crates/wokrouter-platform/tests/fixtures/wokcore-install/wokcore-update-v2.json"
);
const V2_SIGNATURE: &[u8] = include_bytes!(
    "../../../../../crates/wokrouter-platform/tests/fixtures/wokcore-install/wokcore-update-v2.json.minisig"
);
const TEST_TOKEN: &str = "opaque-test-token";

#[derive(Clone, Debug, Eq, PartialEq)]
enum ObservedCall {
    Progress {
        phase: String,
        state: String,
    },
    Connection {
        client: usize,
    },
    Authorize {
        client: usize,
    },
    AuthenticatedStatus {
        client: usize,
        received_authorized_token: bool,
    },
    Stdout(String),
}
#[cfg(windows)]
const WINDOWS_ARCHIVE: &[u8] = &[
    80, 75, 3, 4, 20, 0, 0, 0, 0, 0, 0, 0, 33, 0, 248, 159, 107, 102, 14, 0, 0, 0, 14, 0, 0, 0, 11,
    0, 0, 0, 119, 111, 107, 99, 111, 114, 101, 46, 101, 120, 101, 110, 101, 119, 32, 101, 120, 101,
    99, 117, 116, 97, 98, 108, 101, 80, 75, 1, 2, 20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 33, 0, 248, 159,
    107, 102, 14, 0, 0, 0, 14, 0, 0, 0, 11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 128, 1, 0, 0, 0, 0,
    119, 111, 107, 99, 111, 114, 101, 46, 101, 120, 101, 80, 75, 5, 6, 0, 0, 0, 0, 1, 0, 1, 0, 57,
    0, 0, 0, 55, 0, 0, 0, 0, 0,
];
#[cfg(not(windows))]
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

#[tokio::test]
async fn missing_production_runtime_installs_starts_authorizes_and_reports_structured_progress() {
    let (server, source) = signed_source().await;
    let fixture = tempfile::tempdir().unwrap();
    let paths = app_paths(&fixture);
    let runtime = FakeRuntime::production(None, &paths);
    let service = FakeService::new([
        CoreConnection::Stopped,
        CoreConnection::Running(handshake()),
        CoreConnection::Running(handshake()),
    ]);
    let dependencies = dependencies(source, service.clone());
    let mut output = CapturedOutput::with_calls(service.calls());

    assert_eq!(
        execute_with_dependencies(
            &paths,
            &runtime,
            structured_options(),
            &mut output,
            &dependencies,
        )
        .await,
        Ok(0)
    );

    server.verify().await;
    assert_eq!(service.spawn_count(), 1);
    assert_eq!(service.authorization_count(), 1);
    assert_eq!(service.selected_client(), runtime.client_address());
    assert!(
        service
            .connection_clients()
            .iter()
            .all(|client| *client == runtime.client_address())
    );
    assert_eq!(
        service.authorized_executable(),
        Some(installed_executable(&paths))
    );
    assert_eq!(output.stdout_text(), "{\"code\":\"running\"}\n");
    assert_eq!(output.stdout.len(), 1);

    let events = output.progress_events();
    assert_eq!(
        phases(&events),
        [
            "checking_release",
            "downloading",
            "downloading",
            "verifying",
            "installing",
            "starting",
            "authorizing",
            "verifying_runtime",
            "completed",
        ]
    );
    assert_eq!(
        events
            .iter()
            .map(|event| event["sequence"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        (0..events.len() as u64).collect::<Vec<_>>()
    );
    assert!(events.iter().all(|event| {
        event["schema_version"] == 1
            && event["operation"] == "install"
            && event["state"]
                == if event["phase"] == "completed" {
                    "succeeded"
                } else {
                    "running"
                }
    }));
    let downloads = events
        .iter()
        .filter(|event| event["phase"] == "downloading")
        .collect::<Vec<_>>();
    assert_eq!(downloads.first().unwrap()["bytes_completed"], 0);
    assert_eq!(
        downloads.last().unwrap()["bytes_completed"],
        ARCHIVE.len() as u64
    );
    assert!(downloads.windows(2).all(|pair| {
        pair[0]["bytes_completed"].as_u64() <= pair[1]["bytes_completed"].as_u64()
    }));
    assert!(downloads.iter().all(|event| {
        event["bytes_completed"].as_u64() <= event["bytes_total"].as_u64()
            && event["bytes_total"] == ARCHIVE.len() as u64
    }));
    assert_eq!(
        service.call_suffix(6),
        [
            ObservedCall::Progress {
                phase: "authorizing".to_owned(),
                state: "running".to_owned(),
            },
            ObservedCall::Authorize {
                client: runtime.client_address(),
            },
            ObservedCall::Progress {
                phase: "verifying_runtime".to_owned(),
                state: "running".to_owned(),
            },
            ObservedCall::AuthenticatedStatus {
                client: runtime.client_address(),
                received_authorized_token: true,
            },
            ObservedCall::Progress {
                phase: "completed".to_owned(),
                state: "succeeded".to_owned(),
            },
            ObservedCall::Stdout("{\"code\":\"running\"}\n".to_owned()),
        ]
    );
    assert!(!output.stdout_text().contains(TEST_TOKEN));
    assert!(!output.stderr.concat().contains(TEST_TOKEN));
}

#[tokio::test]
async fn repeated_start_uses_the_trusted_install_record_without_downloading_again() {
    let (server, source) = signed_source().await;
    let fixture = tempfile::tempdir().unwrap();
    let paths = app_paths(&fixture);
    let runtime = FakeRuntime::production(None, &paths);
    let service = FakeService::new([
        CoreConnection::Stopped,
        CoreConnection::Running(handshake()),
        CoreConnection::Running(handshake()),
        CoreConnection::Running(handshake()),
        CoreConnection::Running(handshake()),
    ]);
    let dependencies = dependencies(source, service.clone());

    let mut first = CapturedOutput::default();
    assert_eq!(
        execute_with_dependencies(
            &paths,
            &runtime,
            structured_options(),
            &mut first,
            &dependencies,
        )
        .await,
        Ok(0)
    );
    let mut second = CapturedOutput::default();
    assert_eq!(
        execute_with_dependencies(
            &paths,
            &runtime,
            structured_options(),
            &mut second,
            &dependencies,
        )
        .await,
        Ok(0)
    );

    server.verify().await;
    assert_eq!(service.spawn_count(), 1);
    assert_eq!(service.authorization_count(), 2);
    assert_eq!(second.stdout_text(), "{\"code\":\"already_running\"}\n");
    assert_eq!(
        phases(&second.progress_events()),
        ["authorizing", "verifying_runtime", "completed"]
    );
}

#[tokio::test]
async fn the_first_progress_write_failure_disables_progress_without_changing_the_result() {
    let (server, source) = signed_source().await;
    let fixture = tempfile::tempdir().unwrap();
    let paths = app_paths(&fixture);
    let runtime = FakeRuntime::production(None, &paths);
    let service = FakeService::new([
        CoreConnection::Stopped,
        CoreConnection::Running(handshake()),
        CoreConnection::Running(handshake()),
    ]);
    let dependencies = dependencies(source, service.clone());
    let mut output = CapturedOutput {
        fail_stderr: true,
        ..CapturedOutput::default()
    };

    assert_eq!(
        execute_with_dependencies(
            &paths,
            &runtime,
            structured_options(),
            &mut output,
            &dependencies,
        )
        .await,
        Ok(0)
    );

    server.verify().await;
    assert_eq!(output.stderr_attempts, 1);
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout_text(), "{\"code\":\"running\"}\n");
    assert!(paths.wokcore_install_record.is_file());
    assert_eq!(service.spawn_count(), 1);
    assert_eq!(service.authorization_count(), 1);
}

#[tokio::test]
async fn plain_start_keeps_the_human_message_and_emits_no_progress_jsonl() {
    let (server, source) = signed_source().await;
    let fixture = tempfile::tempdir().unwrap();
    let paths = app_paths(&fixture);
    let runtime = FakeRuntime::production(None, &paths);
    let service = FakeService::new([
        CoreConnection::Stopped,
        CoreConnection::Running(handshake()),
        CoreConnection::Running(handshake()),
    ]);
    let dependencies = dependencies(source, service);
    let mut output = CapturedOutput::default();

    assert_eq!(
        execute_with_dependencies(
            &paths,
            &runtime,
            StartOptions {
                json: false,
                progress_jsonl: false,
            },
            &mut output,
            &dependencies,
        )
        .await,
        Ok(0)
    );

    server.verify().await;
    assert_eq!(output.stdout_text(), "WokCore is running.\n");
    assert!(output.stderr.is_empty());
    assert_eq!(output.stderr_attempts, 0);
}

#[tokio::test]
async fn structured_workflow_failure_owns_terminal_output_and_returns_a_nonzero_code() {
    let fixture = tempfile::tempdir().unwrap();
    let paths = app_paths(&fixture);
    let executable = fixture.path().join("wokcore.exe");
    let runtime = FakeRuntime::production(Some(executable), &paths);
    let service = FakeService::new([CoreConnection::Stopped]);
    service.fail_spawn();
    let dependencies = dependencies(unused_source().await, service);
    let mut output = CapturedOutput::default();

    assert_eq!(
        execute_with_dependencies(
            &paths,
            &runtime,
            structured_options(),
            &mut output,
            &dependencies,
        )
        .await,
        Ok(1)
    );

    assert_eq!(output.stdout_text(), "{\"code\":\"start_failed\"}\n");
    assert_eq!(output.stdout.len(), 1);
    let events = output.progress_events();
    assert_eq!(phases(&events), ["starting", "starting"]);
    assert_eq!(events.last().unwrap()["state"], "failed");
    assert_eq!(events.last().unwrap()["error_code"], "start_failed");
}

#[tokio::test]
async fn structured_authorization_failure_uses_the_stable_terminal_code() {
    let fixture = tempfile::tempdir().unwrap();
    let paths = app_paths(&fixture);
    let runtime = FakeRuntime::production(Some(fixture.path().join("wokcore.exe")), &paths);
    let service = FakeService::new([CoreConnection::Running(handshake())]);
    service.fail_authorization();
    let dependencies = dependencies(unused_source().await, service);
    let mut output = CapturedOutput::default();

    assert_eq!(
        execute_with_dependencies(
            &paths,
            &runtime,
            structured_options(),
            &mut output,
            &dependencies,
        )
        .await,
        Ok(1)
    );

    assert_eq!(
        output.stdout_text(),
        "{\"code\":\"authorization_failed\"}\n"
    );
    assert_eq!(output.stdout.len(), 1);
    let events = output.progress_events();
    assert_eq!(phases(&events), ["authorizing", "authorizing"]);
    assert_eq!(events.last().unwrap()["state"], "failed");
    assert_eq!(events.last().unwrap()["error_code"], "authorization_failed");
}

#[tokio::test]
async fn final_authenticated_non_running_statuses_fail_in_verifying_runtime() {
    for phase in [
        ServicePhase::Starting,
        ServicePhase::Draining,
        ServicePhase::AwaitingCancellation,
        ServicePhase::Stopping,
    ] {
        let fixture = tempfile::tempdir().unwrap();
        let paths = app_paths(&fixture);
        let runtime = FakeRuntime::production(Some(fixture.path().join("wokcore.exe")), &paths);
        let service = FakeService::new([CoreConnection::Running(handshake())]);
        service.set_authenticated_status(Ok(ServiceStatus {
            phase,
            active_requests: 0,
        }));
        let dependencies = dependencies(unused_source().await, service.clone());
        let mut output = CapturedOutput::with_calls(service.calls());

        assert_eq!(
            execute_with_dependencies(
                &paths,
                &runtime,
                structured_options(),
                &mut output,
                &dependencies,
            )
            .await,
            Ok(1),
            "{phase:?}"
        );

        assert_eq!(
            output.stdout_text(),
            "{\"code\":\"start_failed\"}\n",
            "{phase:?}"
        );
        let events = output.progress_events();
        assert_eq!(
            phases(&events),
            ["authorizing", "verifying_runtime", "verifying_runtime"],
            "{phase:?}"
        );
        assert_eq!(events.last().unwrap()["state"], "failed", "{phase:?}");
        assert_eq!(
            events.last().unwrap()["error_code"],
            "start_failed",
            "{phase:?}"
        );
        assert_eq!(
            service.call_suffix(6),
            verification_failure_suffix(runtime.client_address(), "{\"code\":\"start_failed\"}\n",),
            "{phase:?}"
        );
    }
}

#[tokio::test]
async fn final_authenticated_service_errors_have_stable_codes() {
    for (error, expected_code) in [
        (ServiceError::Incompatible, "incompatible_manifest"),
        (ServiceError::InvalidRuntime, "invalid_install_state"),
        (ServiceError::InvalidResponse, "invalid_install_state"),
        (ServiceError::Missing, "start_failed"),
        (ServiceError::Stopped, "start_failed"),
        (ServiceError::Unauthorized, "authorization_failed"),
        (ServiceError::Forbidden, "authorization_failed"),
    ] {
        let fixture = tempfile::tempdir().unwrap();
        let paths = app_paths(&fixture);
        let runtime = FakeRuntime::production(Some(fixture.path().join("wokcore.exe")), &paths);
        let service = FakeService::new([CoreConnection::Running(handshake())]);
        service.set_authenticated_status(Err(error));
        let dependencies = dependencies(unused_source().await, service.clone());
        let mut output = CapturedOutput::with_calls(service.calls());

        assert_eq!(
            execute_with_dependencies(
                &paths,
                &runtime,
                structured_options(),
                &mut output,
                &dependencies,
            )
            .await,
            Ok(1),
            "{error:?}"
        );

        let expected_stdout = format!("{{\"code\":\"{expected_code}\"}}\n");
        assert_eq!(output.stdout_text(), expected_stdout, "{error:?}");
        let events = output.progress_events();
        assert_eq!(
            phases(&events),
            ["authorizing", "verifying_runtime", "verifying_runtime"],
            "{error:?}"
        );
        assert_eq!(events.last().unwrap()["state"], "failed", "{error:?}");
        assert_eq!(
            events.last().unwrap()["error_code"],
            expected_code,
            "{error:?}"
        );
        assert_eq!(
            service.call_suffix(6),
            verification_failure_suffix(runtime.client_address(), &expected_stdout),
            "{error:?}"
        );
    }
}

#[test]
fn invalid_trusted_discovery_has_owned_structured_terminal_output() {
    let mut output = CapturedOutput::default();

    assert_eq!(
        render_structured_platform_error(PlatformError::InvalidWokCoreInstallRecord, &mut output),
        1
    );

    assert_eq!(
        output.stdout_text(),
        "{\"code\":\"invalid_install_state\"}\n"
    );
    assert_eq!(output.stdout.len(), 1);
    let events = output.progress_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["state"], "failed");
    assert_eq!(events[0]["phase"], "checking_release");
    assert_eq!(events[0]["error_code"], "invalid_install_state");
}

#[tokio::test]
async fn running_development_runtime_uses_the_selected_client_without_installing_or_spawning() {
    let (server, source) = source_with_no_expected_requests().await;
    let fixture = tempfile::tempdir().unwrap();
    let paths = app_paths(&fixture);
    let executable = fixture.path().join("wokcore.exe");
    let runtime = FakeRuntime::development(executable.clone(), &paths);
    let service = FakeService::new([
        CoreConnection::Running(handshake()),
        CoreConnection::Running(handshake()),
    ]);
    let dependencies = dependencies(source, service.clone());
    let mut output = CapturedOutput::with_calls(service.calls());

    assert_eq!(
        execute_with_dependencies(
            &paths,
            &runtime,
            structured_options(),
            &mut output,
            &dependencies,
        )
        .await,
        Ok(0)
    );

    server.verify().await;
    assert_eq!(service.spawn_count(), 0);
    assert_eq!(service.authorization_count(), 1);
    assert_eq!(service.selected_client(), runtime.client_address());
    assert_eq!(service.connection_clients(), [runtime.client_address()]);
    assert_eq!(service.authorized_executable(), Some(executable));
    assert_eq!(output.stdout_text(), "{\"code\":\"already_running\"}\n");
    assert_eq!(
        service.call_suffix(6),
        [
            ObservedCall::Progress {
                phase: "authorizing".to_owned(),
                state: "running".to_owned(),
            },
            ObservedCall::Authorize {
                client: runtime.client_address(),
            },
            ObservedCall::Progress {
                phase: "verifying_runtime".to_owned(),
                state: "running".to_owned(),
            },
            ObservedCall::AuthenticatedStatus {
                client: runtime.client_address(),
                received_authorized_token: true,
            },
            ObservedCall::Progress {
                phase: "completed".to_owned(),
                state: "succeeded".to_owned(),
            },
            ObservedCall::Stdout("{\"code\":\"already_running\"}\n".to_owned()),
        ]
    );
}

#[tokio::test]
async fn development_final_unauthorized_status_has_no_install_or_spawn_side_effects() {
    let (server, source) = source_with_no_expected_requests().await;
    let fixture = tempfile::tempdir().unwrap();
    let paths = app_paths(&fixture);
    let runtime = FakeRuntime::development(fixture.path().join("wokcore.exe"), &paths);
    let service = FakeService::new([CoreConnection::Running(handshake())]);
    service.set_authenticated_status(Err(ServiceError::Unauthorized));
    let dependencies = dependencies(source, service.clone());
    let mut output = CapturedOutput::with_calls(service.calls());

    assert_eq!(
        execute_with_dependencies(
            &paths,
            &runtime,
            structured_options(),
            &mut output,
            &dependencies,
        )
        .await,
        Ok(1)
    );

    server.verify().await;
    assert_eq!(service.spawn_count(), 0);
    assert_eq!(service.connection_clients(), [runtime.client_address()]);
    assert_eq!(
        output.stdout_text(),
        "{\"code\":\"authorization_failed\"}\n"
    );
    assert_eq!(
        service.call_suffix(6),
        verification_failure_suffix(
            runtime.client_address(),
            "{\"code\":\"authorization_failed\"}\n",
        )
    );
}

#[tokio::test]
async fn stopped_or_missing_development_runtime_is_left_for_the_ide_without_side_effects() {
    for connection in [CoreConnection::Missing, CoreConnection::Stopped] {
        let (server, source) = source_with_no_expected_requests().await;
        let fixture = tempfile::tempdir().unwrap();
        let paths = app_paths(&fixture);
        let runtime = FakeRuntime::development(fixture.path().join("wokcore.exe"), &paths);
        let service = FakeService::new([connection]);
        let dependencies = dependencies(source, service.clone());
        let mut output = CapturedOutput::default();

        assert_eq!(
            execute_with_dependencies(
                &paths,
                &runtime,
                StartOptions {
                    json: false,
                    progress_jsonl: false,
                },
                &mut output,
                &dependencies,
            )
            .await,
            Err(CommandError::DevelopmentRuntimeManagedByIde)
        );

        server.verify().await;
        assert_eq!(service.spawn_count(), 0);
        assert_eq!(service.authorization_count(), 0);
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
}

#[tokio::test]
async fn a_try_wait_error_kills_and_reaps_the_created_child() {
    let fixture = tempfile::tempdir().unwrap();
    let paths = app_paths(&fixture);
    let runtime = FakeRuntime::production(Some(fixture.path().join("wokcore.exe")), &paths);
    let service = FakeService::new([CoreConnection::Stopped, CoreConnection::Stopped]);
    service.fail_try_wait();
    let dependencies = dependencies(unused_source().await, service.clone());
    let mut output = CapturedOutput::default();

    assert_eq!(
        execute_with_dependencies(
            &paths,
            &runtime,
            structured_options(),
            &mut output,
            &dependencies,
        )
        .await,
        Ok(1)
    );

    assert_eq!(service.kill_count(), 1);
    assert_eq!(service.wait_count(), 1);
    assert_eq!(output.stdout_text(), "{\"code\":\"start_failed\"}\n");
}

#[tokio::test]
async fn final_invalid_runtime_reaps_the_started_child_when_cleanup_try_wait_fails() {
    let fixture = tempfile::tempdir().unwrap();
    let paths = app_paths(&fixture);
    let runtime = FakeRuntime::production(Some(fixture.path().join("wokcore.exe")), &paths);
    let service = FakeService::new([
        CoreConnection::Stopped,
        CoreConnection::Running(handshake()),
        CoreConnection::InvalidRuntime,
    ]);
    service.set_authenticated_status(Err(ServiceError::InvalidRuntime));
    service.fail_try_wait();
    let dependencies = dependencies(unused_source().await, service.clone());
    let mut output = CapturedOutput::default();

    assert_eq!(
        execute_with_dependencies(
            &paths,
            &runtime,
            structured_options(),
            &mut output,
            &dependencies,
        )
        .await,
        Ok(1)
    );

    assert_eq!(service.spawn_count(), 1);
    assert_eq!(service.kill_count(), 1);
    assert_eq!(service.wait_count(), 1);
    assert_eq!(
        output.stdout_text(),
        "{\"code\":\"invalid_install_state\"}\n"
    );
    let events = output.progress_events();
    assert_eq!(
        phases(&events),
        [
            "starting",
            "authorizing",
            "verifying_runtime",
            "verifying_runtime",
        ]
    );
    assert_eq!(events.last().unwrap()["state"], "failed");
    assert_eq!(
        events.last().unwrap()["error_code"],
        "invalid_install_state"
    );
}

#[test]
fn installer_errors_have_the_stable_structured_codes() {
    for (error, expected) in [
        (WokCoreInstallError::InvalidSource, "download_failed"),
        (WokCoreInstallError::DownloadFailed, "download_failed"),
        (
            WokCoreInstallError::InvalidInstallState,
            "invalid_install_state",
        ),
        (
            WokCoreInstallError::InstallInProgress,
            "install_in_progress",
        ),
        (WokCoreInstallError::InvalidManifest, "invalid_manifest"),
        (WokCoreInstallError::InvalidSignature, "invalid_signature"),
        (
            WokCoreInstallError::IncompatibleManifest,
            "incompatible_manifest",
        ),
        (
            WokCoreInstallError::ArtifactSizeMismatch,
            "artifact_size_mismatch",
        ),
        (
            WokCoreInstallError::ArtifactHashMismatch,
            "artifact_hash_mismatch",
        ),
        (WokCoreInstallError::InvalidArchive, "invalid_archive"),
        (
            WokCoreInstallError::UnsafeInstallLocation,
            "unsafe_install_location",
        ),
        (WokCoreInstallError::AtomicInstallFailed, "install_failed"),
        (
            WokCoreInstallError::InstallRecordFailed,
            "install_record_failed",
        ),
    ] {
        assert_eq!(install_error_code(error), expected);
    }
}

#[test]
fn start_process_contains_only_the_fixed_serve_command() {
    use std::ffi::OsStr;

    let executable = Path::new(r"C:\Program Files\WokCore\wokcore.exe");
    let command = spawn_command(executable);

    assert_eq!(command.get_program(), executable.as_os_str());
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        vec![OsStr::new("serve"), OsStr::new("--json")]
    );
}

#[derive(Default)]
struct CapturedOutput {
    stdout: Vec<String>,
    stderr: Vec<String>,
    stderr_attempts: usize,
    fail_stderr: bool,
    calls: Option<Arc<Mutex<Vec<ObservedCall>>>>,
}

impl CapturedOutput {
    fn with_calls(calls: Arc<Mutex<Vec<ObservedCall>>>) -> Self {
        Self {
            calls: Some(calls),
            ..Self::default()
        }
    }

    fn stdout_text(&self) -> String {
        self.stdout.concat()
    }

    fn progress_events(&self) -> Vec<Value> {
        self.stderr
            .iter()
            .flat_map(|write| write.lines())
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

impl StartCommandOutput for CapturedOutput {
    fn stdout(&mut self, value: &str) -> io::Result<()> {
        if let Some(calls) = &self.calls {
            calls
                .lock()
                .unwrap()
                .push(ObservedCall::Stdout(value.to_owned()));
        }
        self.stdout.push(value.to_owned());
        Ok(())
    }

    fn stderr(&mut self, value: &str) -> io::Result<()> {
        self.stderr_attempts += 1;
        if self.fail_stderr {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "synthetic broken progress pipe",
            ));
        }
        if let Some(calls) = &self.calls {
            for line in value.lines() {
                let event: Value = serde_json::from_str(line).unwrap();
                calls.lock().unwrap().push(ObservedCall::Progress {
                    phase: event["phase"].as_str().unwrap().to_owned(),
                    state: event["state"].as_str().unwrap().to_owned(),
                });
            }
        }
        self.stderr.push(value.to_owned());
        Ok(())
    }
}

struct FakeRuntime {
    channel: WokCoreRuntimeChannel,
    executable: Option<PathBuf>,
    client: WokCoreClient,
}

impl FakeRuntime {
    fn production(executable: Option<PathBuf>, paths: &AppPaths) -> Self {
        Self {
            channel: WokCoreRuntimeChannel::Production,
            executable,
            client: WokCoreClient::new(&paths.wokcore_discovery_file).unwrap(),
        }
    }

    fn client_address(&self) -> usize {
        &self.client as *const WokCoreClient as usize
    }

    fn development(executable: PathBuf, paths: &AppPaths) -> Self {
        Self {
            channel: WokCoreRuntimeChannel::Development,
            executable: Some(executable),
            client: WokCoreClient::new(&paths.wokcore_discovery_file).unwrap(),
        }
    }
}

impl CommandRuntime for FakeRuntime {
    fn channel(&self) -> WokCoreRuntimeChannel {
        self.channel
    }

    fn executable(&self) -> Option<&Path> {
        self.executable.as_deref()
    }

    fn client(&self) -> &WokCoreClient {
        &self.client
    }

    async fn connection(&self) -> CoreConnection {
        panic!("the injected service must own connection probing")
    }
}

#[derive(Clone)]
struct FakeService {
    inner: Arc<FakeServiceState>,
}

struct FakeServiceState {
    connections: Mutex<VecDeque<CoreConnection>>,
    last_connection: Mutex<CoreConnection>,
    spawn_count: AtomicUsize,
    authorization_count: AtomicUsize,
    selected_client: AtomicUsize,
    connection_clients: Mutex<Vec<usize>>,
    authenticated_statuses: Mutex<VecDeque<Result<ServiceStatus, ServiceError>>>,
    last_authenticated_status: Mutex<Result<ServiceStatus, ServiceError>>,
    calls: Arc<Mutex<Vec<ObservedCall>>>,
    authorized_executable: Mutex<Option<PathBuf>>,
    fail_spawn: AtomicBool,
    fail_authorization: AtomicBool,
    process: Arc<FakeProcessState>,
}

#[derive(Default)]
struct FakeProcessState {
    fail_try_wait: AtomicBool,
    kill_count: AtomicUsize,
    wait_count: AtomicUsize,
}

impl FakeService {
    fn new(connections: impl IntoIterator<Item = CoreConnection>) -> Self {
        let connections = connections.into_iter().collect::<VecDeque<_>>();
        let last_connection = connections
            .back()
            .cloned()
            .unwrap_or(CoreConnection::Stopped);
        Self {
            inner: Arc::new(FakeServiceState {
                connections: Mutex::new(connections),
                last_connection: Mutex::new(last_connection),
                spawn_count: AtomicUsize::new(0),
                authorization_count: AtomicUsize::new(0),
                selected_client: AtomicUsize::new(0),
                connection_clients: Mutex::new(Vec::new()),
                authenticated_statuses: Mutex::new(VecDeque::new()),
                last_authenticated_status: Mutex::new(Ok(running_status())),
                calls: Arc::new(Mutex::new(Vec::new())),
                authorized_executable: Mutex::new(None),
                fail_spawn: AtomicBool::new(false),
                fail_authorization: AtomicBool::new(false),
                process: Arc::new(FakeProcessState::default()),
            }),
        }
    }

    fn fail_spawn(&self) {
        self.inner.fail_spawn.store(true, Ordering::SeqCst);
    }

    fn fail_try_wait(&self) {
        self.inner
            .process
            .fail_try_wait
            .store(true, Ordering::SeqCst);
    }

    fn fail_authorization(&self) {
        self.inner.fail_authorization.store(true, Ordering::SeqCst);
    }

    fn spawn_count(&self) -> usize {
        self.inner.spawn_count.load(Ordering::SeqCst)
    }

    fn authorization_count(&self) -> usize {
        self.inner.authorization_count.load(Ordering::SeqCst)
    }

    fn selected_client(&self) -> usize {
        self.inner.selected_client.load(Ordering::SeqCst)
    }

    fn connection_clients(&self) -> Vec<usize> {
        self.inner.connection_clients.lock().unwrap().clone()
    }

    fn calls(&self) -> Arc<Mutex<Vec<ObservedCall>>> {
        Arc::clone(&self.inner.calls)
    }

    fn call_suffix(&self, length: usize) -> Vec<ObservedCall> {
        let calls = self.inner.calls.lock().unwrap();
        calls[calls.len() - length..].to_vec()
    }

    fn set_authenticated_status(&self, status: Result<ServiceStatus, ServiceError>) {
        let mut statuses = self.inner.authenticated_statuses.lock().unwrap();
        statuses.clear();
        statuses.push_back(status);
        *self.inner.last_authenticated_status.lock().unwrap() = status;
    }

    fn authorized_executable(&self) -> Option<PathBuf> {
        self.inner.authorized_executable.lock().unwrap().clone()
    }

    fn kill_count(&self) -> usize {
        self.inner.process.kill_count.load(Ordering::SeqCst)
    }

    fn wait_count(&self) -> usize {
        self.inner.process.wait_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl StartService for FakeService {
    async fn connection(&self, client: &WokCoreClient) -> Result<CoreConnection, CommandError> {
        let client = client as *const WokCoreClient as usize;
        self.inner.connection_clients.lock().unwrap().push(client);
        self.inner
            .calls
            .lock()
            .unwrap()
            .push(ObservedCall::Connection { client });
        let connection = self
            .inner
            .connections
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| self.inner.last_connection.lock().unwrap().clone());
        *self.inner.last_connection.lock().unwrap() = connection.clone();
        Ok(connection)
    }

    fn spawn(&self, _executable: &Path) -> Result<Box<dyn StartedCore>, CommandError> {
        self.inner.spawn_count.fetch_add(1, Ordering::SeqCst);
        if self.inner.fail_spawn.load(Ordering::SeqCst) {
            return Err(CommandError::StartFailed);
        }
        Ok(Box::new(FakeStartedCore {
            state: Arc::clone(&self.inner.process),
        }))
    }

    async fn ensure_authorized(
        &self,
        client: &WokCoreClient,
        executable: &Path,
    ) -> Result<SecretString, CommandError> {
        self.inner
            .authorization_count
            .fetch_add(1, Ordering::SeqCst);
        self.inner
            .selected_client
            .store(client as *const WokCoreClient as usize, Ordering::SeqCst);
        self.inner
            .calls
            .lock()
            .unwrap()
            .push(ObservedCall::Authorize {
                client: client as *const WokCoreClient as usize,
            });
        *self.inner.authorized_executable.lock().unwrap() = Some(executable.to_path_buf());
        if self.inner.fail_authorization.load(Ordering::SeqCst) {
            return Err(CommandError::CoreControl);
        }
        Ok(SecretString::from(TEST_TOKEN.to_owned()))
    }

    async fn authenticated_status(
        &self,
        client: &WokCoreClient,
        token: &SecretString,
    ) -> Result<ServiceStatus, ServiceError> {
        let client = client as *const WokCoreClient as usize;
        self.inner
            .calls
            .lock()
            .unwrap()
            .push(ObservedCall::AuthenticatedStatus {
                client,
                received_authorized_token: token.expose_secret() == TEST_TOKEN,
            });
        let status = self
            .inner
            .authenticated_statuses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(*self.inner.last_authenticated_status.lock().unwrap());
        *self.inner.last_authenticated_status.lock().unwrap() = status;
        status
    }
}

struct FakeStartedCore {
    state: Arc<FakeProcessState>,
}

impl StartedCore for FakeStartedCore {
    fn try_wait(&mut self) -> Result<bool, CommandError> {
        if self.state.fail_try_wait.load(Ordering::SeqCst) {
            return Err(CommandError::StartFailed);
        }
        Ok(false)
    }

    fn kill(&mut self) -> Result<(), CommandError> {
        self.state.kill_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn wait(&mut self) -> Result<(), CommandError> {
        self.state.wait_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn dependencies(source: WokCoreInstallSource, service: FakeService) -> StartDependencies {
    StartDependencies {
        install_source: source,
        service: Box::new(service),
    }
}

fn structured_options() -> StartOptions {
    StartOptions {
        json: true,
        progress_jsonl: true,
    }
}

fn phases(events: &[Value]) -> Vec<&str> {
    events
        .iter()
        .map(|event| event["phase"].as_str().unwrap())
        .collect()
}

fn verification_failure_suffix(client: usize, stdout: &str) -> Vec<ObservedCall> {
    vec![
        ObservedCall::Progress {
            phase: "authorizing".to_owned(),
            state: "running".to_owned(),
        },
        ObservedCall::Authorize { client },
        ObservedCall::Progress {
            phase: "verifying_runtime".to_owned(),
            state: "running".to_owned(),
        },
        ObservedCall::AuthenticatedStatus {
            client,
            received_authorized_token: true,
        },
        ObservedCall::Progress {
            phase: "verifying_runtime".to_owned(),
            state: "failed".to_owned(),
        },
        ObservedCall::Stdout(stdout.to_owned()),
    ]
}

async fn signed_source() -> (MockServer, WokCoreInstallSource) {
    let server = MockServer::start().await;
    for (asset_path, body) in [
        ("/releases/wokcore-update-v2.json", V2_MANIFEST),
        ("/releases/wokcore-update-v2.json.minisig", V2_SIGNATURE),
    ] {
        Mock::given(method("GET"))
            .and(path(asset_path))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
            .expect(1)
            .mount(&server)
            .await;
    }
    for asset_path in [
        "/releases/wokcore-update-v1.json",
        "/releases/wokcore-update-v1.json.minisig",
    ] {
        Mock::given(method("GET"))
            .and(path(asset_path))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path(format!("/releases/{}", artifact_name())))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(ARCHIVE))
        .expect(1)
        .mount(&server)
        .await;
    let source = WokCoreInstallSource::loopback(
        format!("{}/releases/", server.uri()).parse().unwrap(),
        PUBLIC_KEY,
    )
    .unwrap();
    (server, source)
}

async fn unused_source() -> WokCoreInstallSource {
    let server = MockServer::start().await;
    WokCoreInstallSource::loopback(
        format!("{}/releases/", server.uri()).parse().unwrap(),
        PUBLIC_KEY,
    )
    .unwrap()
}

async fn source_with_no_expected_requests() -> (MockServer, WokCoreInstallSource) {
    let server = MockServer::start().await;
    for asset_path in [
        "/releases/wokcore-update-v2.json".to_owned(),
        "/releases/wokcore-update-v2.json.minisig".to_owned(),
        "/releases/wokcore-update-v1.json".to_owned(),
        "/releases/wokcore-update-v1.json.minisig".to_owned(),
        format!("/releases/{}", artifact_name()),
    ] {
        Mock::given(method("GET"))
            .and(path(asset_path))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
    }
    let source = WokCoreInstallSource::loopback(
        format!("{}/releases/", server.uri()).parse().unwrap(),
        PUBLIC_KEY,
    )
    .unwrap();
    (server, source)
}

fn artifact_name() -> &'static str {
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

fn app_paths(fixture: &TempDir) -> AppPaths {
    AppPaths {
        config_file: fixture.path().join("config").join("config.toml"),
        wokcore_install_record: fixture.path().join("config").join("wokcore-install.json"),
        wokcore_install_dir: fixture.path().join("WokCore").join("bin"),
        integration_dir: fixture.path().join("state").join("integrations"),
        runtime_dir: fixture.path().join("runtime"),
        log_dir: fixture.path().join("logs"),
        wokcore_discovery_file: fixture.path().join("runtime").join("discovery.json"),
    }
}

fn installed_executable(paths: &AppPaths) -> PathBuf {
    paths
        .wokcore_install_dir
        .join(format!("wokcore{}", std::env::consts::EXE_SUFFIX))
}

fn handshake() -> wokrouter_wokcore_client::CoreHandshake {
    wokrouter_wokcore_client::CoreHandshake {
        instance_id: "test-instance".to_owned(),
        installation_id: None,
        version: "1.2.3".to_owned(),
        management_api_major: 1,
        provider_protocols: Default::default(),
        capabilities: Default::default(),
    }
}

fn running_status() -> ServiceStatus {
    ServiceStatus {
        phase: ServicePhase::Running,
        active_requests: 0,
    }
}
