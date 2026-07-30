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

    fn new_with_selector(selector: Arc<dyn DesktopRuntimeSelector>) -> Self {
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
    use wokrouter_platform::{
        AppPaths, SelectedWokCoreRuntime, WokCoreRuntimeChannel,
        test_support::{RuntimeSelectorHarness, secure_private_file},
    };
    use wokrouter_wokcore_client::CoreConnection;

    use crate::{control::DesktopControl, wokcore::ManagementState};

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
        fixture.write_discovery(41, 2);
        let runtime = development_runtime(&fixture, development).await;
        let selector = Arc::new(CountingSelector::new(runtime));
        let runtime = Arc::new(DesktopRuntimeState::new_with_selector(selector.clone()));
        let control = DesktopControl::new(runtime.clone());
        let management = ManagementState::discover(runtime.clone());

        let (initial_status, management_command) =
            tokio::join!(control.status(), management.command());
        let (_, management_runtime) = management_command.unwrap();

        assert_eq!(
            initial_status.unwrap().runtime_channel,
            WokCoreRuntimeChannel::Development
        );
        assert_eq!(
            management_runtime.channel(),
            WokCoreRuntimeChannel::Development
        );
        assert_eq!(selector.calls.load(Ordering::SeqCst), 1);

        fixture.write_discovery(42, 1);
        fixture.write_install_record(&production);

        let (status, management_command) = tokio::join!(control.status(), management.command());
        let status = status.unwrap();
        let (_, management_runtime) = management_command.unwrap();
        assert_eq!(status.runtime_channel, WokCoreRuntimeChannel::Development);
        assert_eq!(status.state, wokrouter_cli::commands::CoreUiState::Stopped);
        assert_eq!(
            management_runtime.channel(),
            WokCoreRuntimeChannel::Development
        );
        assert_eq!(selector.calls.load(Ordering::SeqCst), 1);

        let encoded = serde_json::to_value(status).unwrap();
        assert_eq!(encoded["runtime_channel"], "development");
        let encoded = encoded.to_string();
        assert!(!encoded.contains("\"pid\""));
        assert!(!encoded.contains("\"path\""));
        assert!(!encoded.contains("\"executable\""));
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
            fs::write(
                &self.paths.wokcore_discovery_file,
                serde_json::to_vec(&serde_json::json!({
                    "base_url": "http://127.0.0.1:9",
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
