use std::{future::Future, pin::Pin, sync::Arc};

use tokio::sync::OnceCell;
use wokrouter_platform::{AppPaths, SelectedWokCoreRuntime, select_wokcore_runtime};

type RuntimeSelectionFuture<'a> =
    Pin<Box<dyn Future<Output = Result<SelectedWokCoreRuntime, DesktopRuntimeError>> + Send + 'a>>;

pub(crate) trait DesktopRuntimeSelector: Send + Sync {
    fn select(&self) -> RuntimeSelectionFuture<'_>;
}

struct SystemDesktopRuntimeSelector;

impl DesktopRuntimeSelector for SystemDesktopRuntimeSelector {
    fn select(&self) -> RuntimeSelectionFuture<'_> {
        Box::pin(async {
            let paths = AppPaths::discover().map_err(|_| DesktopRuntimeError::Initialization)?;
            select_wokcore_runtime(&paths)
                .await
                .map_err(|_| DesktopRuntimeError::Initialization)
        })
    }
}

pub(crate) struct DesktopRuntimeState {
    selected: OnceCell<Result<SelectedWokCoreRuntime, DesktopRuntimeError>>,
    selector: Arc<dyn DesktopRuntimeSelector>,
}

impl DesktopRuntimeState {
    pub(crate) fn discover() -> Self {
        Self::new_with_selector(Arc::new(SystemDesktopRuntimeSelector))
    }

    pub(crate) fn new_with_selector(selector: Arc<dyn DesktopRuntimeSelector>) -> Self {
        Self {
            selected: OnceCell::new(),
            selector,
        }
    }

    pub(crate) async fn selected(&self) -> Result<&SelectedWokCoreRuntime, DesktopRuntimeError> {
        self.selected
            .get_or_init(|| self.selector.select())
            .await
            .as_ref()
            .map_err(|error| *error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum DesktopRuntimeError {
    #[error("runtime_initialization_failed")]
    Initialization,
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsStr,
        fs,
        future::Future,
        path::{Path, PathBuf},
        pin::Pin,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use tempfile::{TempDir, tempdir};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path},
    };
    use wokrouter_platform::{
        AppPaths, SelectedWokCoreRuntime, WokCoreRuntimeChannel,
        test_support::{RuntimeSelectorHarness, secure_private_file},
    };
    use wokrouter_wokcore_client::CoreConnection;

    use crate::{
        control::{DesktopControl, DesktopLifecycle, LifecycleFuture},
        wokcore::{ManagementState, provider_catalog_inner},
    };

    use super::{DesktopRuntimeError, DesktopRuntimeSelector, DesktopRuntimeState};

    struct CountingSelector {
        calls: AtomicUsize,
        runtime: Mutex<Option<SelectedWokCoreRuntime>>,
    }

    impl CountingSelector {
        fn new(runtime: SelectedWokCoreRuntime) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                runtime: Mutex::new(Some(runtime)),
            }
        }
    }

    impl DesktopRuntimeSelector for CountingSelector {
        fn select(
            &self,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<SelectedWokCoreRuntime, DesktopRuntimeError>>
                    + Send
                    + '_,
            >,
        > {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let runtime = self
                .runtime
                .lock()
                .unwrap()
                .as_ref()
                .expect("test runtime is present")
                .clone();
            Box::pin(async move { Ok(runtime) })
        }
    }

    struct FailingSelector {
        calls: AtomicUsize,
    }

    impl DesktopRuntimeSelector for FailingSelector {
        fn select(
            &self,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<SelectedWokCoreRuntime, DesktopRuntimeError>>
                    + Send
                    + '_,
            >,
        > {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err(DesktopRuntimeError::Initialization) })
        }
    }

    #[tokio::test]
    async fn concurrent_callers_share_one_selected_runtime_reference() {
        let fixture = RuntimeFixture::new();
        let development = fixture.create_file("development/wokcore");
        fixture.write_discovery(41, 2);
        let runtime = development_runtime(&fixture, development).await;
        let selector = Arc::new(CountingSelector::new(runtime));
        let state = DesktopRuntimeState::new_with_selector(selector.clone());

        let (first, second) = tokio::join!(state.selected(), state.selected());

        let first = first.unwrap();
        let second = second.unwrap();
        assert!(std::ptr::eq(first, second));
        assert_eq!(first.channel(), WokCoreRuntimeChannel::Development);
        assert_eq!(selector.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn selected_development_runtime_never_switches_to_replacement_production_discovery() {
        let fixture = RuntimeFixture::new();
        let development = fixture.create_file("development/wokcore");
        let production = fixture.create_file("production/wokcore");
        fixture.write_discovery(41, 2);
        let runtime = development_runtime(&fixture, development).await;
        let selector = Arc::new(CountingSelector::new(runtime));
        let state = DesktopRuntimeState::new_with_selector(selector.clone());
        let selected = state.selected().await.unwrap();
        assert_eq!(selected.channel(), WokCoreRuntimeChannel::Development);

        fixture.write_discovery(42, 1);
        fixture.write_install_record(&production);

        let selected_after_replacement = state.selected().await.unwrap();
        assert!(std::ptr::eq(selected, selected_after_replacement));
        assert_eq!(
            selected_after_replacement.channel(),
            WokCoreRuntimeChannel::Development
        );
        assert_eq!(
            selected_after_replacement.connection().await,
            CoreConnection::Stopped
        );
        assert_eq!(selector.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn status_and_management_share_one_selection_and_remain_on_development() {
        let fixture = RuntimeFixture::new();
        let development = fixture.create_file("development/wokcore");
        let production = fixture.create_file("production/wokcore");
        let development_server = MockServer::start().await;
        let production_server = MockServer::start().await;
        mount_handshake(&development_server).await;
        mount_provider_catalog(&development_server).await;
        mount_handshake(&production_server).await;
        mount_provider_catalog(&production_server).await;
        fixture.write_discovery_url(41, 1, &development_server.uri());
        let runtime = development_runtime_for_process(&fixture, development, 41).await;
        let selector = Arc::new(CountingSelector::new(runtime));
        let runtime = Arc::new(DesktopRuntimeState::new_with_selector(selector.clone()));
        let control = DesktopControl::new(runtime.clone());
        let management = ManagementState::for_test(
            runtime.clone(),
            fixture.root.path().join("exports"),
            TEST_TOKEN,
        );

        let (initial_status, catalog) =
            tokio::join!(control.status(), provider_catalog_inner(&management));

        assert_eq!(
            initial_status.unwrap().runtime_channel,
            WokCoreRuntimeChannel::Development
        );
        assert_eq!(catalog.unwrap().baseline_commit, "development");
        assert_eq!(selector.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            request_count(&development_server, "/wokcore/v1/providers/catalog").await,
            1
        );

        fixture.write_discovery_url(42, 1, &production_server.uri());
        fixture.write_install_record(&production);

        let (status, catalog) = tokio::join!(control.status(), provider_catalog_inner(&management));
        let status = status.unwrap();
        assert_eq!(status.runtime_channel, WokCoreRuntimeChannel::Development);
        assert_eq!(status.state, wokrouter_cli::commands::CoreUiState::Stopped);
        assert_eq!(
            serde_json::to_value(catalog.unwrap_err()).unwrap()["code"],
            "runtime_missing"
        );
        assert_eq!(selector.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            request_count(&development_server, "/wokcore/v1/providers/catalog").await,
            1
        );
        assert_eq!(
            request_count(&production_server, "/wokcore/v1/providers/catalog").await,
            0
        );

        let encoded = serde_json::to_value(status).unwrap();
        assert_eq!(encoded["runtime_channel"], "development");
        let encoded = encoded.to_string();
        assert!(!encoded.contains("\"pid\""));
        assert!(!encoded.contains("\"path\""));
        assert!(!encoded.contains("\"executable\""));
    }

    #[tokio::test]
    async fn lifecycle_actions_share_the_selected_production_runtime_and_retry_after_failure() {
        let fixture = RuntimeFixture::new();
        let production = fixture.create_file("production/wokcore");
        fixture.write_install_record(&production);
        let selected = production_runtime(&fixture, production).await;
        let selector = Arc::new(CountingSelector::new(selected));
        let runtime = Arc::new(DesktopRuntimeState::new_with_selector(selector.clone()));
        let lifecycle = Arc::new(FakeLifecycle::new([
            Err(wokrouter_cli::commands::CommandError::StartFailed),
            Ok(0),
        ]));
        let control = DesktopControl::new_with_lifecycle(runtime, lifecycle.clone());

        assert_eq!(
            control.start().await.unwrap_err().to_string(),
            "start_unavailable"
        );
        assert!(control.start().await.is_ok());
        assert!(control.stop().await.is_ok());

        assert_eq!(lifecycle.start_calls.load(Ordering::SeqCst), 2);
        assert_eq!(lifecycle.stop_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            lifecycle.channels.lock().unwrap().as_slice(),
            [
                WokCoreRuntimeChannel::Production,
                WokCoreRuntimeChannel::Production,
                WokCoreRuntimeChannel::Production,
            ]
        );
        assert_eq!(selector.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn concurrent_starts_delegate_once_and_share_the_same_result() {
        let fixture = RuntimeFixture::new();
        let production = fixture.create_file("production/wokcore");
        fixture.write_install_record(&production);
        let selected = production_runtime(&fixture, production).await;
        let selector = Arc::new(CountingSelector::new(selected));
        let runtime = Arc::new(DesktopRuntimeState::new_with_selector(selector.clone()));
        let lifecycle = Arc::new(FakeLifecycle::new([Ok(0)]));
        let control = DesktopControl::new_with_lifecycle(runtime, lifecycle.clone());

        let (first, second) = tokio::join!(control.start(), control.start());

        assert_eq!(first, Ok(()));
        assert_eq!(second, Ok(()));
        assert_eq!(lifecycle.start_calls.load(Ordering::SeqCst), 1);
        assert_eq!(selector.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn lifecycle_command_errors_map_to_stable_desktop_codes() {
        let fixture = RuntimeFixture::new();
        let production = fixture.create_file("production/wokcore");
        fixture.write_install_record(&production);
        let selected = production_runtime(&fixture, production).await;
        let selector = Arc::new(CountingSelector::new(selected));
        let runtime = Arc::new(DesktopRuntimeState::new_with_selector(selector.clone()));
        let lifecycle = Arc::new(
            FakeLifecycle::new([Err(
                wokrouter_cli::commands::CommandError::DevelopmentRuntimeManagedByIde,
            )])
            .with_stop_result(Err(wokrouter_cli::commands::CommandError::StopTimedOut)),
        );
        let control = DesktopControl::new_with_lifecycle(runtime, lifecycle);

        assert_eq!(
            control.start().await.unwrap_err().to_string(),
            "development_runtime_managed_by_ide"
        );
        assert_eq!(
            control.stop().await.unwrap_err().to_string(),
            "stop_unavailable"
        );
        assert_eq!(selector.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn stopped_development_lifecycle_actions_return_the_stable_ide_managed_code() {
        let fixture = RuntimeFixture::new();
        let development = fixture.create_file("development/wokcore");
        fixture.write_discovery(41, 2);
        let runtime = development_runtime(&fixture, development).await;
        let state = Arc::new(DesktopRuntimeState::new_with_selector(Arc::new(
            CountingSelector::new(runtime),
        )));
        let control = DesktopControl::new(state);

        fixture.write_discovery(42, 1);

        assert_eq!(
            control.start().await.unwrap_err().to_string(),
            "development_runtime_managed_by_ide"
        );
        assert_eq!(
            control.stop().await.unwrap_err().to_string(),
            "development_runtime_managed_by_ide"
        );
    }

    #[tokio::test]
    async fn selection_failure_is_cached_as_one_stable_error() {
        let selector = Arc::new(FailingSelector {
            calls: AtomicUsize::new(0),
        });
        let state = DesktopRuntimeState::new_with_selector(selector.clone());

        let (first, second) = tokio::join!(state.selected(), state.selected());

        assert!(matches!(first, Err(DesktopRuntimeError::Initialization)));
        assert!(matches!(second, Err(DesktopRuntimeError::Initialization)));
        assert_eq!(selector.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            DesktopRuntimeError::Initialization.to_string(),
            "runtime_initialization_failed"
        );
    }

    async fn development_runtime(
        fixture: &RuntimeFixture,
        development: PathBuf,
    ) -> SelectedWokCoreRuntime {
        RuntimeSelectorHarness::new(
            Some(development.into_os_string()),
            |_process_id, _candidate| true,
            |_record| panic!("production discovery must not run"),
        )
        .select(&fixture.paths)
        .await
        .unwrap()
    }

    async fn development_runtime_for_process(
        fixture: &RuntimeFixture,
        development: PathBuf,
        expected_process_id: u32,
    ) -> SelectedWokCoreRuntime {
        RuntimeSelectorHarness::new(
            Some(development.clone().into_os_string()),
            move |process_id, candidate| {
                process_id.get() == expected_process_id && candidate == development
            },
            |_record| panic!("production discovery must not run"),
        )
        .select(&fixture.paths)
        .await
        .unwrap()
    }

    async fn production_runtime(
        fixture: &RuntimeFixture,
        production: PathBuf,
    ) -> SelectedWokCoreRuntime {
        RuntimeSelectorHarness::new(
            None,
            |_process_id, _candidate| false,
            move |_record| Ok(Some(production.clone())),
        )
        .select(&fixture.paths)
        .await
        .unwrap()
    }

    const TEST_TOKEN: &str = "wok_proxy_v1_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    async fn mount_handshake(server: &MockServer) {
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
                "capabilities": ["providers.read"],
                "instance_id": "01234567-89ab-4cde-8fab-0123456789ab",
                "installation_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            })))
            .mount(server)
            .await;
    }

    async fn mount_provider_catalog(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/wokcore/v1/providers/catalog"))
            .and(header(
                "authorization",
                format!("Bearer {TEST_TOKEN}").as_str(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "schema_version": 1,
                "catalog_schema_version": 1,
                "baseline_commit": "development",
                "providers": [{
                    "id": "synthetic",
                    "label": "Synthetic",
                    "adapter": "open_ai_responses",
                    "base_url": "https://example.invalid/v1",
                    "auth_kind": "key",
                    "endpoint_policy": "public_https",
                    "model_source": "static",
                    "aliases": [],
                    "models": ["synthetic-model"],
                    "default_model": "synthetic-model",
                    "allow_endpoint_override": false,
                    "key_optional": false,
                    "allow_key_auth_override": false,
                    "reasoning_efforts": [],
                    "reasoning_effort_map": {},
                    "capabilities": {
                        "text": true,
                        "streaming": true,
                        "tools": false,
                        "vision": false,
                        "images": false,
                        "reasoning": false
                    }
                }]
            })))
            .mount(server)
            .await;
    }

    async fn request_count(server: &MockServer, request_path: &str) -> usize {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.url.path() == request_path)
            .count()
    }

    struct FakeLifecycle {
        starts:
            Mutex<std::collections::VecDeque<Result<u8, wokrouter_cli::commands::CommandError>>>,
        stop_result: Mutex<Option<Result<u8, wokrouter_cli::commands::CommandError>>>,
        start_calls: AtomicUsize,
        stop_calls: AtomicUsize,
        channels: Mutex<Vec<WokCoreRuntimeChannel>>,
    }

    impl FakeLifecycle {
        fn new(
            starts: impl IntoIterator<Item = Result<u8, wokrouter_cli::commands::CommandError>>,
        ) -> Self {
            Self {
                starts: Mutex::new(starts.into_iter().collect()),
                stop_result: Mutex::new(Some(Ok(0))),
                start_calls: AtomicUsize::new(0),
                stop_calls: AtomicUsize::new(0),
                channels: Mutex::new(Vec::new()),
            }
        }

        fn with_stop_result(
            self,
            result: Result<u8, wokrouter_cli::commands::CommandError>,
        ) -> Self {
            *self.stop_result.lock().unwrap() = Some(result);
            self
        }
    }

    impl DesktopLifecycle for FakeLifecycle {
        fn start<'a>(&'a self, runtime: &'a SelectedWokCoreRuntime) -> LifecycleFuture<'a> {
            self.start_calls.fetch_add(1, Ordering::SeqCst);
            self.channels.lock().unwrap().push(runtime.channel());
            let result = self.starts.lock().unwrap().pop_front().unwrap();
            Box::pin(async move { result })
        }

        fn stop<'a>(&'a self, runtime: &'a SelectedWokCoreRuntime) -> LifecycleFuture<'a> {
            self.stop_calls.fetch_add(1, Ordering::SeqCst);
            self.channels.lock().unwrap().push(runtime.channel());
            let result = self.stop_result.lock().unwrap().take().unwrap();
            Box::pin(async move { result })
        }
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
            self.write_discovery_url(process_id, api_major, "http://127.0.0.1:9");
        }

        fn write_discovery_url(&self, process_id: u32, api_major: u32, base_url: &str) {
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
        } else {
            path.to_owned()
        }
    }
}
