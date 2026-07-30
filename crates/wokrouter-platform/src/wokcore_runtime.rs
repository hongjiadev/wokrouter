use std::{
    future::Future,
    path::{Path, PathBuf},
};

use serde::Serialize;
use tokio::sync::OnceCell;
use wokrouter_wokcore_client::{CoreConnection, WokCoreClient};

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

#[derive(Clone, Debug)]
pub struct SelectedWokCoreRuntime {
    channel: WokCoreRuntimeChannel,
    executable: Option<PathBuf>,
    client: WokCoreClient,
}

impl SelectedWokCoreRuntime {
    pub fn channel(&self) -> WokCoreRuntimeChannel {
        self.channel
    }

    pub fn executable(&self) -> Option<&Path> {
        self.executable.as_deref()
    }

    pub fn client(&self) -> &WokCoreClient {
        &self.client
    }

    pub async fn connection(&self) -> CoreConnection {
        match (self.channel, self.client.connection().await) {
            (WokCoreRuntimeChannel::Development, CoreConnection::Missing) => {
                CoreConnection::Stopped
            }
            (_, connection) => connection,
        }
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
        &crate::system::process_executable_matches,
        &discover_wokcore_executable,
    )
    .await
}

#[cfg(not(debug_assertions))]
async fn select_once(paths: &AppPaths) -> Result<SelectedWokCoreRuntime, PlatformError> {
    select_production(paths, &discover_wokcore_executable)
}

fn client(paths: &AppPaths) -> Result<WokCoreClient, PlatformError> {
    WokCoreClient::new(&paths.wokcore_discovery_file)
        .map_err(|_| PlatformError::WokCoreClientInitialization)
}

fn select_production(
    paths: &AppPaths,
    discover: &(dyn Fn(&Path) -> Result<Option<PathBuf>, PlatformError> + Send + Sync),
) -> Result<SelectedWokCoreRuntime, PlatformError> {
    Ok(SelectedWokCoreRuntime {
        channel: WokCoreRuntimeChannel::Production,
        executable: discover(&paths.wokcore_install_record)?,
        client: client(paths)?,
    })
}

#[cfg(debug_assertions)]
async fn select_with_dependencies(
    paths: &AppPaths,
    candidate: Option<PathBuf>,
    process_matches: &(dyn Fn(std::num::NonZeroU32, &Path) -> bool + Send + Sync),
    discover: &(dyn Fn(&Path) -> Result<Option<PathBuf>, PlatformError> + Send + Sync),
) -> Result<SelectedWokCoreRuntime, PlatformError> {
    use std::time::Duration;

    use tokio::time::Instant;

    const DEVELOPMENT_TIMEOUT: Duration = Duration::from_secs(5);
    const DEVELOPMENT_RETRY_DELAY: Duration = Duration::from_millis(50);

    let Some(candidate) = candidate else {
        return select_production(paths, discover);
    };
    let client = client(paths)?;
    let deadline = Instant::now() + DEVELOPMENT_TIMEOUT;
    loop {
        if let Some(process_id) = client.discovered_process_id()
            && process_matches(process_id, &candidate)
        {
            let bound = client.bound_to_process(process_id);
            let Ok(connection) = tokio::time::timeout_at(deadline, bound.connection()).await else {
                break;
            };
            if Instant::now() >= deadline {
                break;
            }
            let still_matches = process_matches(process_id, &candidate);
            if still_matches && !matches!(connection, CoreConnection::Missing) {
                return Ok(SelectedWokCoreRuntime {
                    channel: WokCoreRuntimeChannel::Development,
                    executable: Some(candidate),
                    client: bound,
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
    select_production(paths, discover)
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
        num::NonZeroU32,
        path::{Path, PathBuf},
        sync::Arc,
    };

    use super::{
        RuntimeSelectorState, SelectedWokCoreRuntime, development, select_with_dependencies,
    };
    use crate::{AppPaths, PlatformError};

    type ProcessMatcher = dyn Fn(NonZeroU32, &Path) -> bool + Send + Sync;
    type ProductionDiscoverer =
        dyn Fn(&Path) -> Result<Option<PathBuf>, PlatformError> + Send + Sync;

    #[derive(Clone)]
    pub struct RuntimeSelectorHarness {
        inner: Arc<RuntimeSelector>,
    }

    struct RuntimeSelector {
        state: RuntimeSelectorState,
        candidate: Option<PathBuf>,
        process_matches: Arc<ProcessMatcher>,
        discover: Arc<ProductionDiscoverer>,
    }

    impl RuntimeSelectorHarness {
        pub fn new(
            candidate: Option<OsString>,
            process_matches: impl Fn(NonZeroU32, &Path) -> bool + Send + Sync + 'static,
            discover: impl Fn(&Path) -> Result<Option<PathBuf>, PlatformError> + Send + Sync + 'static,
        ) -> Self {
            Self {
                inner: Arc::new(RuntimeSelector {
                    state: RuntimeSelectorState::new(),
                    candidate: development::candidate_from_value(candidate),
                    process_matches: Arc::new(process_matches),
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
                        self.inner.process_matches.as_ref(),
                        self.inner.discover.as_ref(),
                    )
                })
                .await
        }
    }
}
