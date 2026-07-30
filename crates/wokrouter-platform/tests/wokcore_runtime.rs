use std::{
    ffi::{OsStr, OsString},
    fs,
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tempfile::{TempDir, tempdir};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};
use wokrouter_platform::{
    AppPaths, WokCoreRuntimeChannel,
    test_support::{
        RuntimeSelectorHarness, process_executable_matches, secure_private_directory,
        secure_private_file,
    },
};
use wokrouter_wokcore_client::CoreConnection;

static ENVIRONMENT_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn runtime_channel_serializes_as_snake_case() {
    assert_eq!(
        serde_json::to_value(WokCoreRuntimeChannel::Development).unwrap(),
        "development"
    );
    assert_eq!(
        serde_json::to_value(WokCoreRuntimeChannel::Production).unwrap(),
        "production"
    );
}

#[tokio::test(start_paused = true)]
async fn absent_or_invalid_development_candidates_select_production_immediately() {
    for candidate in [
        None,
        Some(OsString::new()),
        Some(OsString::from("relative/wokcore")),
    ] {
        let fixture = RuntimeFixture::new();
        let production = fixture.create_file("production/wokcore");
        let discoveries = Arc::new(AtomicUsize::new(0));
        let selector = selector(
            candidate,
            false,
            Some(production.clone()),
            Arc::clone(&discoveries),
        );
        let started = tokio::time::Instant::now();

        let selected = selector.select(&fixture.paths).await.unwrap();

        assert_eq!(selected.channel(), WokCoreRuntimeChannel::Production);
        assert_eq!(selected.executable(), Some(production.as_path()));
        assert_eq!(tokio::time::Instant::now() - started, Duration::ZERO);
        assert_eq!(discoveries.load(Ordering::SeqCst), 1);
    }

    let fixture = RuntimeFixture::new();
    let wrong_name = fixture.create_file("development/not-wokcore");
    let production = fixture.create_file("production/wokcore");
    let discoveries = Arc::new(AtomicUsize::new(0));
    let selector = selector(
        Some(wrong_name.into_os_string()),
        true,
        Some(production),
        Arc::clone(&discoveries),
    );
    let started = tokio::time::Instant::now();

    let selected = selector.select(&fixture.paths).await.unwrap();

    assert_eq!(selected.channel(), WokCoreRuntimeChannel::Production);
    assert_eq!(tokio::time::Instant::now() - started, Duration::ZERO);
    assert_eq!(discoveries.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn missing_discovery_falls_back_to_production_at_five_second_deadline() {
    let fixture = RuntimeFixture::new();
    let development = fixture.create_file("development/wokcore");
    let production = fixture.create_file("production/wokcore");
    let discoveries = Arc::new(AtomicUsize::new(0));
    let selector = selector(
        Some(development.into_os_string()),
        true,
        Some(production),
        Arc::clone(&discoveries),
    );
    let started = tokio::time::Instant::now();

    let selected = selector.select(&fixture.paths).await.unwrap();
    let elapsed = tokio::time::Instant::now() - started;

    assert_eq!(
        selected.channel(),
        WokCoreRuntimeChannel::Production,
        "selection completed after {elapsed:?}"
    );
    assert_eq!(elapsed, Duration::from_secs(5));
    assert_eq!(discoveries.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn a_wrong_process_image_is_never_selected_as_development() {
    let fixture = RuntimeFixture::new();
    let development = fixture.create_file("development/wokcore");
    let production = fixture.create_file("production/wokcore");
    fixture.write_discovery(41, 2);
    let discoveries = Arc::new(AtomicUsize::new(0));
    let selector = selector(
        Some(development.into_os_string()),
        false,
        Some(production),
        Arc::clone(&discoveries),
    );
    let started = tokio::time::Instant::now();

    let selected = selector.select(&fixture.paths).await.unwrap();

    assert_eq!(selected.channel(), WokCoreRuntimeChannel::Production);
    assert_eq!(
        tokio::time::Instant::now() - started,
        Duration::from_secs(5)
    );
    assert_eq!(discoveries.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn a_matching_process_that_appears_before_the_deadline_selects_development() {
    let fixture = RuntimeFixture::new();
    let development = fixture.create_file("development/wokcore");
    fixture.write_discovery(41, 2);
    let matches = Arc::new(AtomicUsize::new(0));
    let matcher_arguments = Arc::new(Mutex::new(Vec::new()));
    let discoveries = Arc::new(AtomicUsize::new(0));
    let selector = RuntimeSelectorHarness::new(
        Some(development.clone().into_os_string()),
        {
            let matches = Arc::clone(&matches);
            let matcher_arguments = Arc::clone(&matcher_arguments);
            move |process_id, candidate| {
                matcher_arguments
                    .lock()
                    .unwrap()
                    .push((process_id.get(), candidate.to_owned()));
                matches.fetch_add(1, Ordering::SeqCst) >= 1
            }
        },
        {
            let discoveries = Arc::clone(&discoveries);
            move |_record| {
                discoveries.fetch_add(1, Ordering::SeqCst);
                Ok(None)
            }
        },
    );
    let started = tokio::time::Instant::now();

    let selected = selector.select(&fixture.paths).await.unwrap();

    assert_eq!(selected.channel(), WokCoreRuntimeChannel::Development);
    assert_eq!(selected.executable(), Some(development.as_path()));
    assert_eq!(
        tokio::time::Instant::now() - started,
        Duration::from_millis(50)
    );
    assert_eq!(
        *matcher_arguments.lock().unwrap(),
        vec![
            (41, development.clone()),
            (41, development.clone()),
            (41, development.clone())
        ]
    );
    assert_eq!(discoveries.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn development_selection_rechecks_the_same_process_identity_after_connection() {
    let fixture = RuntimeFixture::new();
    let development = fixture.create_file("development/wokcore");
    let production = fixture.create_file("production/wokcore");
    fixture.write_discovery(41, 2);
    let matcher_arguments = Arc::new(Mutex::new(Vec::new()));
    let discoveries = Arc::new(AtomicUsize::new(0));
    let discovery_file = fixture.paths.wokcore_discovery_file.clone();
    let selector = RuntimeSelectorHarness::new(
        Some(development.clone().into_os_string()),
        {
            let matcher_arguments = Arc::clone(&matcher_arguments);
            move |process_id, candidate| {
                let mut arguments = matcher_arguments.lock().unwrap();
                arguments.push((process_id.get(), candidate.to_owned()));
                if arguments.len() == 2 {
                    fs::remove_file(&discovery_file).unwrap();
                }
                arguments.len() == 1
            }
        },
        {
            let discoveries = Arc::clone(&discoveries);
            move |_record| {
                discoveries.fetch_add(1, Ordering::SeqCst);
                Ok(Some(production.clone()))
            }
        },
    );

    let selected = selector.select(&fixture.paths).await.unwrap();

    assert_eq!(selected.channel(), WokCoreRuntimeChannel::Production);
    assert_eq!(
        *matcher_arguments.lock().unwrap(),
        vec![(41, development.clone()), (41, development)]
    );
    assert_eq!(discoveries.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn a_matching_incompatible_runtime_stays_on_the_development_channel() {
    let fixture = RuntimeFixture::new();
    let development = fixture.create_file("development/wokcore");
    fixture.write_discovery(41, 2);
    let discoveries = Arc::new(AtomicUsize::new(0));
    let selector = selector(
        Some(development.clone().into_os_string()),
        true,
        None,
        Arc::clone(&discoveries),
    );

    let selected = selector.select(&fixture.paths).await.unwrap();

    assert_eq!(selected.channel(), WokCoreRuntimeChannel::Development);
    assert_eq!(selected.executable(), Some(development.as_path()));
    assert!(matches!(
        selected.connection().await,
        CoreConnection::Incompatible(_)
    ));
    assert_eq!(discoveries.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_matching_invalid_runtime_stays_on_the_development_channel() {
    let fixture = RuntimeFixture::new();
    let development = fixture.create_file("development/wokcore");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wokcore/v1/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "invalid",
            "instance_id": "01234567-89ab-4cde-8fab-0123456789ab"
        })))
        .mount(&server)
        .await;
    fixture.write_discovery_at(41, 1, &server.uri());
    let discoveries = Arc::new(AtomicUsize::new(0));
    let selector = selector(
        Some(development.clone().into_os_string()),
        true,
        None,
        Arc::clone(&discoveries),
    );

    let selected = selector.select(&fixture.paths).await.unwrap();

    assert_eq!(selected.channel(), WokCoreRuntimeChannel::Development);
    assert_eq!(selected.executable(), Some(development.as_path()));
    assert_eq!(selected.connection().await, CoreConnection::InvalidRuntime);
    assert_eq!(discoveries.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn development_connection_probing_never_runs_past_the_five_second_deadline() {
    let fixture = RuntimeFixture::new();
    let development = fixture.create_file("development/wokcore");
    let production = fixture.create_file("production/wokcore");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/wokcore/v1/health"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(10))
                .set_body_json(serde_json::json!({
                    "status": "ok",
                    "instance_id": "01234567-89ab-4cde-8fab-0123456789ab"
                })),
        )
        .mount(&server)
        .await;
    fixture.write_discovery_at(41, 1, &server.uri());
    let matches = Arc::new(AtomicUsize::new(0));
    let discoveries = Arc::new(AtomicUsize::new(0));
    let selector = RuntimeSelectorHarness::new(
        Some(development.into_os_string()),
        {
            let matches = Arc::clone(&matches);
            move |_process_id, _candidate| {
                matches.fetch_add(1, Ordering::SeqCst);
                true
            }
        },
        {
            let discoveries = Arc::clone(&discoveries);
            move |_record| {
                discoveries.fetch_add(1, Ordering::SeqCst);
                Ok(Some(production.clone()))
            }
        },
    );
    let started = tokio::time::Instant::now();

    let paths = fixture.paths.clone();
    let selection = tokio::spawn(async move { selector.select(&paths).await });
    let mut requests = Vec::new();
    for _ in 0..100 {
        tokio::task::yield_now().await;
        requests = server.received_requests().await.unwrap();
        if !requests.is_empty() {
            break;
        }
    }
    assert_eq!(requests.len(), 1, "slow connection request never started");
    assert_eq!(requests[0].url.path(), "/wokcore/v1/health");

    let deadline = started + Duration::from_secs(5);
    let now = tokio::time::Instant::now();
    assert!(
        now < deadline,
        "slow request did not start before {deadline:?}"
    );
    tokio::time::advance(deadline - now).await;
    let selected = selection.await.unwrap().unwrap();
    let elapsed = tokio::time::Instant::now() - started;

    assert_eq!(
        selected.channel(),
        WokCoreRuntimeChannel::Production,
        "selection completed after {elapsed:?}"
    );
    assert_eq!(elapsed, Duration::from_secs(5));
    assert_eq!(matches.load(Ordering::SeqCst), 1);
    assert_eq!(discoveries.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn concurrent_calls_share_one_selection_attempt() {
    let fixture = RuntimeFixture::new();
    let development = fixture.create_file("development/wokcore");
    let production = fixture.create_file("production/wokcore");
    let discoveries = Arc::new(AtomicUsize::new(0));
    let selector = selector(
        Some(development.into_os_string()),
        true,
        Some(production),
        Arc::clone(&discoveries),
    );

    let (first, second) = tokio::join!(
        selector.select(&fixture.paths),
        selector.select(&fixture.paths)
    );

    assert_eq!(first.unwrap().channel(), WokCoreRuntimeChannel::Production);
    assert_eq!(second.unwrap().channel(), WokCoreRuntimeChannel::Production);
    assert_eq!(discoveries.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_selected_development_session_never_switches_to_production() {
    let fixture = RuntimeFixture::new();
    let development = fixture.create_file("development/wokcore");
    fixture.write_discovery(41, 2);
    let discoveries = Arc::new(AtomicUsize::new(0));
    let selector = selector(
        Some(development.clone().into_os_string()),
        true,
        None,
        Arc::clone(&discoveries),
    );
    let selected = selector.select(&fixture.paths).await.unwrap();

    let replacement = MockServer::start().await;
    mount_running_runtime(&replacement).await;
    fixture.write_discovery_at(42, 1, &replacement.uri());

    assert_eq!(selected.channel(), WokCoreRuntimeChannel::Development);
    assert_eq!(selected.executable(), Some(development.as_path()));
    assert_eq!(selected.connection().await, CoreConnection::Stopped);
    assert!(replacement.received_requests().await.unwrap().is_empty());
    assert_eq!(discoveries.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn production_selection_preserves_install_record_priority_over_path() {
    let fixture = RuntimeFixture::new();
    let managed = fixture.create_file("managed/wokcore");
    let path_candidate = fixture.create_file("path/wokcore");
    make_production_executable(&managed);
    make_production_executable(&path_candidate);
    secure_private_directory(path_candidate.parent().unwrap()).unwrap();
    fixture.write_install_record(&managed);
    let path_directory = path_candidate.parent().unwrap().to_owned();
    let selector = RuntimeSelectorHarness::new(
        None,
        |_process_id, _candidate| false,
        move |record| {
            let _environment = ENVIRONMENT_LOCK.lock().unwrap();
            let _path =
                EnvironmentGuard::set("PATH", std::env::join_paths([&path_directory]).unwrap());
            wokrouter_platform::discover_wokcore_executable(record)
        },
    );

    let selected = selector.select(&fixture.paths).await.unwrap();

    assert_eq!(selected.channel(), WokCoreRuntimeChannel::Production);
    assert_eq!(selected.executable(), Some(managed.as_path()));
}

#[test]
fn process_matching_compares_file_identity_and_rejects_reparse_points() {
    let current_executable = std::env::current_exe().unwrap();
    let process_id = NonZeroU32::new(std::process::id()).unwrap();
    assert!(process_executable_matches(process_id, &current_executable));

    let fixture = tempdir().unwrap();
    let copy = fixture.path().join(current_executable.file_name().unwrap());
    fs::copy(&current_executable, &copy).unwrap();
    assert!(!process_executable_matches(process_id, &copy));

    let link = fixture.path().join("linked-process-image");
    if create_file_symlink(&current_executable, &link).is_ok() {
        assert!(!process_executable_matches(process_id, &link));
    }
}

fn selector(
    candidate: Option<OsString>,
    matches: bool,
    production: Option<PathBuf>,
    discoveries: Arc<AtomicUsize>,
) -> RuntimeSelectorHarness {
    RuntimeSelectorHarness::new(
        candidate,
        move |_process_id, _candidate| matches,
        move |_record| {
            discoveries.fetch_add(1, Ordering::SeqCst);
            Ok(production.clone())
        },
    )
}

async fn mount_running_runtime(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/wokcore/v1/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "ok",
            "instance_id": "01234567-89ab-4cde-8fab-0123456789ab"
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/wokcore/v1/capabilities"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "wokcore_version": "0.1.0",
            "management_api_major": 1,
            "minimum_management_api_major": 1,
            "maximum_management_api_major": 1,
            "provider_protocols": ["openai_responses"],
            "capabilities": ["discovery.v1"],
            "instance_id": "01234567-89ab-4cde-8fab-0123456789ab"
        })))
        .mount(server)
        .await;
}

struct RuntimeFixture {
    root: TempDir,
    paths: AppPaths,
}

impl RuntimeFixture {
    fn new() -> Self {
        let root = tempdir().unwrap();
        let paths = AppPaths {
            config_file: root.path().join("config.toml"),
            wokcore_install_record: root.path().join("wokcore-install.json"),
            wokcore_install_dir: root.path().join("managed"),
            integration_dir: root.path().join("integrations"),
            runtime_dir: root.path().join("runtime"),
            log_dir: root.path().join("logs"),
            wokcore_discovery_file: root.path().join("discovery.json"),
        };
        Self { root, paths }
    }

    fn create_file(&self, relative: &str) -> PathBuf {
        let relative = platform_executable_path(relative);
        let path = self.root.path().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"synthetic executable").unwrap();
        path
    }

    fn write_discovery(&self, process_id: u32, api_major: u32) {
        self.write_discovery_at(process_id, api_major, "http://127.0.0.1:9");
    }

    fn write_discovery_at(&self, process_id: u32, api_major: u32, base_url: &str) {
        fs::write(
            &self.paths.wokcore_discovery_file,
            serde_json::to_vec(&serde_json::json!({
                "base_url": base_url,
                "pid": process_id,
                "instance_id": "01234567-89ab-4cde-8fab-0123456789ab",
                "wokcore_version": "0.1.0",
                "api_major": api_major
            }))
            .unwrap(),
        )
        .unwrap();
        secure_private_file(&self.paths.wokcore_discovery_file).unwrap();
    }

    fn write_install_record(&self, executable: &Path) {
        fs::write(
            &self.paths.wokcore_install_record,
            serde_json::to_vec(&serde_json::json!({
                "schema_version": 1,
                "executable": executable
            }))
            .unwrap(),
        )
        .unwrap();
        secure_private_file(&self.paths.wokcore_install_record).unwrap();
    }
}

fn platform_executable_path(relative: &str) -> PathBuf {
    let path = Path::new(relative);
    if path.file_name() == Some(OsStr::new("wokcore")) {
        path.with_file_name(format!("wokcore{}", std::env::consts::EXE_SUFFIX))
    } else if path.file_name() == Some(OsStr::new("not-wokcore")) {
        path.with_file_name(format!("not-wokcore{}", std::env::consts::EXE_SUFFIX))
    } else {
        path.to_owned()
    }
}

#[cfg(unix)]
fn make_production_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

#[cfg(windows)]
fn make_production_executable(path: &Path) {
    secure_private_file(path).unwrap();
}

#[cfg(unix)]
fn create_file_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_file_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

struct EnvironmentGuard {
    name: &'static str,
    original: Option<OsString>,
}

impl EnvironmentGuard {
    fn set(name: &'static str, value: OsString) -> Self {
        let original = std::env::var_os(name);
        unsafe {
            std::env::set_var(name, value);
        }
        Self { name, original }
    }
}

impl Drop for EnvironmentGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.original {
                Some(value) => std::env::set_var(self.name, value),
                None => std::env::remove_var(self.name),
            }
        }
    }
}
