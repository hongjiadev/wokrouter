use std::{
    future::Future,
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use serde::Serialize;
use tokio::sync::OnceCell;
use wokrouter_wokcore_client::{CoreConnection, WokCoreClient, WokCoreRuntimeBinder};

use crate::{AppPaths, PlatformError, system::wokcore::discover_wokcore_executable};

static SELECTED_RUNTIME: RuntimeSelectorState = RuntimeSelectorState::new();

struct RuntimeSelectorState {
    selected: OnceCell<SelectedWokCoreRuntime>,
}

impl RuntimeSelectorState {
    const fn new() -> Self {
        Self {
            selected: OnceCell::const_new(),
        }
    }

    async fn select<F, Fut>(&self, initialize: F) -> Result<SelectedWokCoreRuntime, PlatformError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<SelectedWokCoreRuntime, PlatformError>>,
    {
        self.selected.get_or_try_init(initialize).await.cloned()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WokCoreRuntimeChannel {
    Development,
    Production,
}

type ProcessMatcher = dyn Fn(NonZeroU32, &Path) -> bool + Send + Sync;
type ProductionDiscoverer = dyn Fn(&Path) -> Result<Option<PathBuf>, PlatformError> + Send + Sync;

#[derive(Clone)]
pub struct SelectedWokCoreRuntime {
    channel: WokCoreRuntimeChannel,
    executable: Arc<OnceLock<PathBuf>>,
    client: WokCoreClient,
    probe_client: WokCoreClient,
    runtime_binder: Option<WokCoreRuntimeBinder>,
    process_matches: Arc<ProcessMatcher>,
    production_authority: Option<(PathBuf, Arc<ProductionDiscoverer>)>,
}

impl SelectedWokCoreRuntime {
    pub fn channel(&self) -> WokCoreRuntimeChannel {
        self.channel
    }

    pub fn executable(&self) -> Option<&Path> {
        self.executable.get().map(PathBuf::as_path)
    }

    pub fn client(&self) -> &WokCoreClient {
        self.refresh_production_binding();
        &self.client
    }

    pub async fn connection(&self) -> CoreConnection {
        match (self.channel, self.client().connection().await) {
            (WokCoreRuntimeChannel::Development, CoreConnection::Missing) => {
                CoreConnection::Stopped
            }
            (_, connection) => connection,
        }
    }

    pub fn establish_production_binding(&self, executable: &Path) -> bool {
        if self.channel != WokCoreRuntimeChannel::Production {
            return false;
        }
        if let Some(selected) = self.executable.get() {
            if selected != executable {
                return false;
            }
        } else if self.executable.set(executable.to_path_buf()).is_err() {
            return false;
        }

        let Some(binder) = &self.runtime_binder else {
            return false;
        };
        let Some(identity) = self.probe_client.discovered_runtime_identity() else {
            return false;
        };
        if !(self.process_matches)(identity.process_id(), executable) {
            return false;
        }
        bind_trusted_executable(
            binder,
            Arc::clone(&self.process_matches),
            executable.to_path_buf(),
        )
    }

    fn refresh_production_binding(&self) {
        let Some((install_record, discover)) = &self.production_authority else {
            return;
        };
        let executable = match self.executable() {
            Some(executable) => executable.to_path_buf(),
            None => match discover(install_record) {
                Ok(Some(executable)) => executable,
                Ok(None) | Err(_) => return,
            },
        };
        let _ = self.establish_production_binding(&executable);
    }
}

impl std::fmt::Debug for SelectedWokCoreRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SelectedWokCoreRuntime")
            .field("channel", &self.channel)
            .field("executable", &self.executable())
            .finish_non_exhaustive()
    }
}

pub async fn select_wokcore_runtime(
    paths: &AppPaths,
) -> Result<SelectedWokCoreRuntime, PlatformError> {
    SELECTED_RUNTIME.select(|| select_once(paths)).await
}

#[cfg(debug_assertions)]
async fn select_once(paths: &AppPaths) -> Result<SelectedWokCoreRuntime, PlatformError> {
    let candidate = development::candidate_from_environment();
    select_with_dependencies(
        paths,
        candidate,
        Arc::new(crate::system::process_executable_matches),
        &probe_connection,
        Arc::new(discover_wokcore_executable),
    )
    .await
}

#[cfg(not(debug_assertions))]
async fn select_once(paths: &AppPaths) -> Result<SelectedWokCoreRuntime, PlatformError> {
    select_production(
        paths,
        Arc::new(crate::system::process_executable_matches),
        Arc::new(discover_wokcore_executable),
    )
}

fn client(paths: &AppPaths) -> Result<WokCoreClient, PlatformError> {
    WokCoreClient::new(&paths.wokcore_discovery_file)
        .map_err(|_| PlatformError::WokCoreClientInitialization)
}

fn select_production(
    paths: &AppPaths,
    process_matches: Arc<ProcessMatcher>,
    discover: Arc<ProductionDiscoverer>,
) -> Result<SelectedWokCoreRuntime, PlatformError> {
    let probe_client = client(paths)?;
    let (bound_client, runtime_binder) = probe_client.pending_trusted_executable_runtime();
    let executable = discover(&paths.wokcore_install_record)?;
    let executable_cell = Arc::new(OnceLock::new());
    if let Some(executable) = executable {
        let _ = executable_cell.set(executable.clone());
        bind_trusted_executable(&runtime_binder, Arc::clone(&process_matches), executable);
    }
    Ok(SelectedWokCoreRuntime {
        channel: WokCoreRuntimeChannel::Production,
        executable: executable_cell,
        client: bound_client,
        probe_client,
        runtime_binder: Some(runtime_binder),
        process_matches,
        production_authority: Some((paths.wokcore_install_record.clone(), Arc::clone(&discover))),
    })
}

fn bind_trusted_executable(
    binder: &WokCoreRuntimeBinder,
    process_matches: Arc<ProcessMatcher>,
    executable: PathBuf,
) -> bool {
    binder.bind_trusted_executable(Arc::new(move |process_id| {
        process_matches(process_id, &executable)
    }))
}

#[cfg(debug_assertions)]
type ConnectionProbeFuture =
    std::pin::Pin<Box<dyn Future<Output = CoreConnection> + Send + 'static>>;
#[cfg(debug_assertions)]
type ConnectionProbe = dyn Fn(WokCoreClient) -> ConnectionProbeFuture + Send + Sync;

#[cfg(debug_assertions)]
fn probe_connection(client: WokCoreClient) -> ConnectionProbeFuture {
    Box::pin(async move { client.connection().await })
}

#[cfg(debug_assertions)]
async fn select_with_dependencies(
    paths: &AppPaths,
    candidate: Option<PathBuf>,
    process_matches: Arc<ProcessMatcher>,
    connection_probe: &ConnectionProbe,
    discover: Arc<ProductionDiscoverer>,
) -> Result<SelectedWokCoreRuntime, PlatformError> {
    use std::time::Duration;

    use tokio::time::Instant;

    const DEVELOPMENT_TIMEOUT: Duration = Duration::from_secs(5);
    const DEVELOPMENT_RETRY_DELAY: Duration = Duration::from_millis(50);

    let Some(candidate) = candidate else {
        return select_production(paths, process_matches, discover);
    };
    let client = client(paths)?;
    let deadline = Instant::now() + DEVELOPMENT_TIMEOUT;
    loop {
        if let Some(identity) = client.discovered_runtime_identity()
            && process_matches(identity.process_id(), &candidate)
        {
            let candidate_for_validator = candidate.clone();
            let matcher_for_validator = Arc::clone(&process_matches);
            let bound = client.bound_to_runtime(
                identity,
                Arc::new(move |process_id| {
                    matcher_for_validator(process_id, &candidate_for_validator)
                }),
            );
            let Ok(connection) =
                tokio::time::timeout_at(deadline, connection_probe(bound.clone())).await
            else {
                break;
            };
            if Instant::now() >= deadline {
                break;
            }
            let still_matches = client.discovered_runtime_identity() == Some(identity)
                && process_matches(identity.process_id(), &candidate);
            if still_matches && !matches!(connection, CoreConnection::Missing) {
                let executable = Arc::new(OnceLock::new());
                let _ = executable.set(candidate);
                return Ok(SelectedWokCoreRuntime {
                    channel: WokCoreRuntimeChannel::Development,
                    executable,
                    client: bound,
                    probe_client: client,
                    runtime_binder: None,
                    process_matches,
                    production_authority: None,
                });
            }
        }

        let now = Instant::now();
        if now >= deadline {
            break;
        }
        tokio::time::sleep(DEVELOPMENT_RETRY_DELAY.min(deadline - now)).await;
        if Instant::now() >= deadline {
            break;
        }
    }
    select_production(paths, process_matches, discover)
}

#[cfg(debug_assertions)]
mod development {
    use std::{
        ffi::OsString,
        path::{Path, PathBuf},
    };

    pub(super) const EXECUTABLE_ENV: &str = "WOKROUTER_DEV_WOKCORE_EXECUTABLE";

    pub(super) fn candidate_from_environment() -> Option<PathBuf> {
        candidate_from_value(std::env::var_os(EXECUTABLE_ENV))
    }

    pub(super) fn candidate_from_value(value: Option<OsString>) -> Option<PathBuf> {
        value
            .map(PathBuf::from)
            .filter(|path| valid_candidate(path))
    }

    fn valid_candidate(path: &Path) -> bool {
        if !path.is_absolute() || path.file_name() != Some(executable_name()) {
            return false;
        }
        let Ok(metadata) = std::fs::symlink_metadata(path) else {
            return false;
        };
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && valid_platform_metadata(&metadata)
    }

    #[cfg(windows)]
    fn executable_name() -> &'static std::ffi::OsStr {
        std::ffi::OsStr::new("wokcore.exe")
    }

    #[cfg(not(windows))]
    fn executable_name() -> &'static std::ffi::OsStr {
        std::ffi::OsStr::new("wokcore")
    }

    #[cfg(windows)]
    fn valid_platform_metadata(metadata: &std::fs::Metadata) -> bool {
        use std::os::windows::fs::MetadataExt;

        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
    }

    #[cfg(not(windows))]
    fn valid_platform_metadata(_metadata: &std::fs::Metadata) -> bool {
        true
    }
}

#[cfg(all(feature = "test-support", debug_assertions))]
pub(crate) mod test_support {
    use std::{
        ffi::OsString,
        future::Future,
        num::NonZeroU32,
        path::{Path, PathBuf},
        sync::Arc,
    };

    use super::{
        ConnectionProbe, ConnectionProbeFuture, ProcessMatcher, ProductionDiscoverer,
        RuntimeSelectorState, SelectedWokCoreRuntime, development, probe_connection,
        select_with_dependencies,
    };
    use crate::{AppPaths, PlatformError};
    use wokrouter_wokcore_client::{CoreConnection, WokCoreClient};

    #[derive(Clone)]
    pub struct RuntimeSelectorHarness {
        inner: Arc<RuntimeSelector>,
    }

    struct RuntimeSelector {
        state: RuntimeSelectorState,
        candidate: Option<PathBuf>,
        process_matches: Arc<ProcessMatcher>,
        connection_probe: Arc<ConnectionProbe>,
        discover: Arc<ProductionDiscoverer>,
    }

    impl RuntimeSelectorHarness {
        pub fn new(
            candidate: Option<OsString>,
            process_matches: impl Fn(NonZeroU32, &Path) -> bool + Send + Sync + 'static,
            discover: impl Fn(&Path) -> Result<Option<PathBuf>, PlatformError> + Send + Sync + 'static,
        ) -> Self {
            Self::new_with_connection_probe(candidate, process_matches, discover, probe_connection)
        }

        pub fn new_with_connection_probe<Probe, ProbeFuture>(
            candidate: Option<OsString>,
            process_matches: impl Fn(NonZeroU32, &Path) -> bool + Send + Sync + 'static,
            discover: impl Fn(&Path) -> Result<Option<PathBuf>, PlatformError> + Send + Sync + 'static,
            connection_probe: Probe,
        ) -> Self
        where
            Probe: Fn(WokCoreClient) -> ProbeFuture + Send + Sync + 'static,
            ProbeFuture: Future<Output = CoreConnection> + Send + 'static,
        {
            let connection_probe =
                move |client| -> ConnectionProbeFuture { Box::pin(connection_probe(client)) };
            Self {
                inner: Arc::new(RuntimeSelector {
                    state: RuntimeSelectorState::new(),
                    candidate: development::candidate_from_value(candidate),
                    process_matches: Arc::new(process_matches),
                    connection_probe: Arc::new(connection_probe),
                    discover: Arc::new(discover),
                }),
            }
        }

        pub async fn select(
            &self,
            paths: &AppPaths,
        ) -> Result<SelectedWokCoreRuntime, PlatformError> {
            self.inner
                .state
                .select(|| {
                    select_with_dependencies(
                        paths,
                        self.inner.candidate.clone(),
                        Arc::clone(&self.inner.process_matches),
                        self.inner.connection_probe.as_ref(),
                        Arc::clone(&self.inner.discover),
                    )
                })
                .await
        }
    }
}
