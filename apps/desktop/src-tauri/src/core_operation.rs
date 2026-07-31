mod helper;
mod journal;
mod parser;

use std::{
    env, future::Future, path::PathBuf, pin::Pin, process::Stdio, sync::Arc, time::Duration,
};

use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    sync::{Mutex, mpsc},
};
use uuid::Uuid;
use wokrouter_cli::commands::{CoreUiState, status::snapshot_selected};
use wokrouter_platform::{
    AppPaths, PlatformError, WokCoreRuntimeChannel, discover_wokcore_executable,
};

use self::{
    journal::{JournalLease, LeaseAttempt, OperationJournal},
    parser::{ChildProgress, MAX_BUFFER_BYTES, ProgressParser},
};
use crate::runtime::DesktopRuntimeState;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CoreOperationKind {
    Install,
    Update,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CoreOperationState {
    Running,
    Succeeded,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CoreOperationPhase {
    CheckingRelease,
    Downloading,
    Verifying,
    Installing,
    PreparingService,
    Draining,
    Stopping,
    Starting,
    Authorizing,
    VerifyingRuntime,
    RollingBack,
    Completed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CoreOperationSnapshot {
    pub schema_version: u8,
    pub operation_id: Uuid,
    pub sequence: u64,
    pub operation: CoreOperationKind,
    pub state: CoreOperationState,
    pub phase: CoreOperationPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_completed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_requests: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

impl CoreOperationSnapshot {
    fn initial(operation: CoreOperationKind) -> Self {
        Self {
            schema_version: 1,
            operation_id: Uuid::new_v4(),
            sequence: 0,
            operation,
            state: CoreOperationState::Running,
            phase: CoreOperationPhase::CheckingRelease,
            current_version: None,
            target_version: None,
            bytes_completed: None,
            bytes_total: None,
            active_requests: None,
            error_code: None,
        }
    }

    fn from_child(operation_id: Uuid, sequence: u64, child: ChildProgress) -> Self {
        Self {
            schema_version: child.schema_version,
            operation_id,
            sequence,
            operation: child.operation,
            state: child.state,
            phase: child.phase,
            current_version: child.current_version,
            target_version: child.target_version,
            bytes_completed: child.bytes_completed,
            bytes_total: child.bytes_total,
            active_requests: child.active_requests,
            error_code: child.error_code,
        }
    }

    fn failed(
        operation_id: Uuid,
        sequence: u64,
        operation: CoreOperationKind,
        error_code: &'static str,
    ) -> Self {
        Self {
            schema_version: 1,
            operation_id,
            sequence,
            operation,
            state: CoreOperationState::Failed,
            phase: CoreOperationPhase::Completed,
            current_version: None,
            target_version: None,
            bytes_completed: None,
            bytes_total: None,
            active_requests: None,
            error_code: Some(error_code.to_owned()),
        }
    }

    fn is_safe_projection(&self) -> bool {
        if self.schema_version != 1
            || !phase_is_valid(self.operation, self.phase)
            || !versions_are_bounded(self)
            || !bytes_are_valid(self)
            || self.active_requests.is_some_and(|count| {
                self.operation != CoreOperationKind::Update || count > 1_000_000
            })
        {
            return false;
        }
        match self.state {
            CoreOperationState::Running => {
                self.phase != CoreOperationPhase::Completed && self.error_code.is_none()
            }
            CoreOperationState::Succeeded => {
                self.phase == CoreOperationPhase::Completed && self.error_code.is_none()
            }
            CoreOperationState::Failed => self
                .error_code
                .as_deref()
                .is_some_and(|code| error_code_is_valid(self.operation, code)),
        }
    }
}

fn phase_is_valid(operation: CoreOperationKind, phase: CoreOperationPhase) -> bool {
    match operation {
        CoreOperationKind::Install => matches!(
            phase,
            CoreOperationPhase::CheckingRelease
                | CoreOperationPhase::Downloading
                | CoreOperationPhase::Verifying
                | CoreOperationPhase::Installing
                | CoreOperationPhase::Starting
                | CoreOperationPhase::Authorizing
                | CoreOperationPhase::VerifyingRuntime
                | CoreOperationPhase::Completed
        ),
        CoreOperationKind::Update => matches!(
            phase,
            CoreOperationPhase::CheckingRelease
                | CoreOperationPhase::Downloading
                | CoreOperationPhase::Verifying
                | CoreOperationPhase::Installing
                | CoreOperationPhase::PreparingService
                | CoreOperationPhase::Draining
                | CoreOperationPhase::Stopping
                | CoreOperationPhase::Starting
                | CoreOperationPhase::VerifyingRuntime
                | CoreOperationPhase::RollingBack
                | CoreOperationPhase::Completed
        ),
    }
}

fn versions_are_bounded(snapshot: &CoreOperationSnapshot) -> bool {
    [&snapshot.current_version, &snapshot.target_version]
        .into_iter()
        .flatten()
        .all(|value| {
            value.len() <= 64
                && value.is_ascii()
                && Version::parse(value).is_ok_and(|version| version.to_string() == *value)
        })
}

fn bytes_are_valid(snapshot: &CoreOperationSnapshot) -> bool {
    match (
        snapshot.phase,
        snapshot.bytes_completed,
        snapshot.bytes_total,
    ) {
        (CoreOperationPhase::Downloading, Some(completed), Some(total)) => {
            total > 0 && completed <= total
        }
        (CoreOperationPhase::Downloading, _, _) => false,
        (_, None, None) => true,
        (_, _, _) => false,
    }
}

fn error_code_is_valid(operation: CoreOperationKind, code: &str) -> bool {
    match operation {
        CoreOperationKind::Install => matches!(
            code,
            "download_failed"
                | "invalid_install_state"
                | "invalid_manifest"
                | "invalid_signature"
                | "incompatible_manifest"
                | "artifact_size_mismatch"
                | "artifact_hash_mismatch"
                | "invalid_archive"
                | "unsafe_install_location"
                | "install_in_progress"
                | "install_failed"
                | "install_record_failed"
                | "start_failed"
                | "authorization_failed"
                | "invalid_progress"
        ),
        CoreOperationKind::Update => matches!(
            code,
            "update_unavailable"
                | "incompatible_manifest"
                | "update_verification_failed"
                | "update_install_failed"
                | "active_requests_remain"
                | "rolled_back"
                | "recovery_required"
                | "operation_in_progress"
                | "invalid_progress"
        ),
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct CoreUpdateCheck {
    pub code: String,
    pub current_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_version: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum CoreOperationError {
    #[error("runtime_initialization_failed")]
    Initialization,
    #[error("development_runtime_managed_by_ide")]
    DevelopmentRuntimeManagedByIde,
    #[error("operation_in_progress")]
    OperationInProgress,
    #[error("update_unavailable")]
    UpdateUnavailable,
    #[error("invalid_install_state")]
    InvalidInstallState,
    #[error("update_verification_failed")]
    UpdateVerificationFailed,
    #[error("invalid_progress")]
    InvalidProgress,
}

#[cfg(test)]
impl CoreOperationError {
    fn code(self) -> &'static str {
        match self {
            Self::Initialization => "runtime_initialization_failed",
            Self::DevelopmentRuntimeManagedByIde => "development_runtime_managed_by_ide",
            Self::OperationInProgress => "operation_in_progress",
            Self::UpdateUnavailable => "update_unavailable",
            Self::InvalidInstallState => "invalid_install_state",
            Self::UpdateVerificationFailed => "update_verification_failed",
            Self::InvalidProgress => "invalid_progress",
        }
    }
}

pub(crate) type EventFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

pub(crate) trait OperationEventSink: Send + Sync {
    fn emit<'a>(&'a self, snapshot: &'a CoreOperationSnapshot) -> EventFuture<'a>;
}

type OperationFuture =
    Pin<Box<dyn Future<Output = Result<ChildCompletion, RunnerError>> + Send + 'static>>;
type CheckFuture =
    Pin<Box<dyn Future<Output = Result<CheckCompletion, RunnerError>> + Send + 'static>>;

trait OperationRunner: Send + Sync {
    fn run(
        self: Arc<Self>,
        request: OperationRequest,
        progress: mpsc::Sender<ChildProgress>,
    ) -> OperationFuture;

    fn check_update(self: Arc<Self>, executable: PathBuf) -> CheckFuture;
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum OperationRequest {
    Install,
    Update { executable: PathBuf },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RunnerError {
    Spawn,
    Wait,
    Read,
    InvalidOutput,
}

struct ChildCompletion {
    exit_success: bool,
    stdout: Vec<u8>,
    progress_valid: bool,
    last_progress: Option<ChildProgress>,
}

#[cfg(test)]
impl ChildCompletion {
    fn with_progress<const N: usize>(
        exit_success: bool,
        stdout: &[u8],
        progress: [ChildProgress; N],
    ) -> Result<Self, RunnerError> {
        Ok(Self {
            exit_success,
            stdout: stdout.to_vec(),
            progress_valid: true,
            last_progress: progress.into_iter().last(),
        })
    }

    fn progress(&self) -> impl Iterator<Item = &ChildProgress> {
        self.last_progress.iter()
    }
}

struct CheckCompletion {
    exit_success: bool,
    stdout: Vec<u8>,
}

impl CheckCompletion {
    #[cfg(test)]
    fn success(stdout: &[u8]) -> Self {
        Self {
            exit_success: true,
            stdout: stdout.to_vec(),
        }
    }
}

trait TrustedRuntimeAuthority: Send + Sync {
    fn discover(&self) -> Result<Option<PathBuf>, CoreOperationError>;
}

type RecoveryFuture<'a> =
    Pin<Box<dyn Future<Output = Result<RecoveryRuntimeState, CoreOperationError>> + Send + 'a>>;

#[derive(Clone, Debug, Eq, PartialEq)]
enum RecoveryRuntimeState {
    Missing,
    Ready { version: Option<String> },
    Unavailable,
}

trait OperationRecoveryProbe: Send + Sync {
    fn runtime(&self) -> RecoveryFuture<'_>;
    fn install_lease_active(&self) -> Result<bool, CoreOperationError>;
}

struct SystemOperationRecoveryProbe {
    runtime: Arc<DesktopRuntimeState>,
    install_directory: PathBuf,
}

impl OperationRecoveryProbe for SystemOperationRecoveryProbe {
    fn runtime(&self) -> RecoveryFuture<'_> {
        Box::pin(async move {
            let runtime = self
                .runtime
                .selected()
                .await
                .map_err(|_| CoreOperationError::Initialization)?;
            if runtime.channel() != WokCoreRuntimeChannel::Production {
                return Ok(RecoveryRuntimeState::Unavailable);
            }
            let (status, _) = snapshot_selected(runtime)
                .await
                .map_err(|_| CoreOperationError::Initialization)?;
            Ok(match status.state {
                CoreUiState::Missing => RecoveryRuntimeState::Missing,
                CoreUiState::Running => RecoveryRuntimeState::Ready {
                    version: status.version,
                },
                _ => RecoveryRuntimeState::Unavailable,
            })
        })
    }

    fn install_lease_active(&self) -> Result<bool, CoreOperationError> {
        wokrouter_platform::wokcore_install_lease_active(&self.install_directory)
            .map_err(|_| CoreOperationError::Initialization)
    }
}

trait HelperLauncher: Send + Sync {
    fn launch(
        &self,
        operation_id: Uuid,
        operation: CoreOperationKind,
    ) -> Result<tokio::process::Child, RunnerError>;
}

struct SystemHelperLauncher;

impl HelperLauncher for SystemHelperLauncher {
    fn launch(
        &self,
        operation_id: Uuid,
        operation: CoreOperationKind,
    ) -> Result<tokio::process::Child, RunnerError> {
        let executable = env::current_exe().map_err(|_| RunnerError::Spawn)?;
        helper::spawn_helper_process(&executable, operation_id, operation)
    }
}

fn reap_helper(mut helper: tokio::process::Child) {
    tauri::async_runtime::spawn(async move {
        let _ = helper.wait().await;
    });
}

enum HelperTimeoutResolution {
    Active(CoreOperationSnapshot),
    Fenced(JournalLease),
}

fn fence_helper_timeout(
    journal: &OperationJournal,
    expected: &CoreOperationSnapshot,
) -> Result<HelperTimeoutResolution, CoreOperationError> {
    match journal.try_operation_lease()? {
        LeaseAttempt::Busy => {
            let current = journal.read()?.ok_or(CoreOperationError::InvalidProgress)?;
            if current.operation_id != expected.operation_id
                || current.operation != expected.operation
            {
                return Err(CoreOperationError::InvalidProgress);
            }
            Ok(HelperTimeoutResolution::Active(current))
        }
        LeaseAttempt::Acquired(fence) => {
            let current = journal.read()?.ok_or(CoreOperationError::InvalidProgress)?;
            if current.operation_id != expected.operation_id
                || current.operation != expected.operation
            {
                return Err(CoreOperationError::InvalidProgress);
            }
            if current != *expected {
                drop(fence);
                return Ok(HelperTimeoutResolution::Active(current));
            }
            Ok(HelperTimeoutResolution::Fenced(fence))
        }
    }
}

pub(crate) fn run_operation_helper_if_requested() -> Option<u8> {
    helper::run_operation_helper_if_requested()
}

struct SystemTrustedRuntimeAuthority {
    paths: Option<AppPaths>,
}

impl SystemTrustedRuntimeAuthority {
    fn discover() -> Self {
        Self { paths: None }
    }

    #[cfg(test)]
    fn from_paths(paths: AppPaths) -> Self {
        Self { paths: Some(paths) }
    }
}

impl TrustedRuntimeAuthority for SystemTrustedRuntimeAuthority {
    fn discover(&self) -> Result<Option<PathBuf>, CoreOperationError> {
        let paths = match &self.paths {
            Some(paths) => paths.clone(),
            None => AppPaths::discover().map_err(|_| CoreOperationError::Initialization)?,
        };
        discover_wokcore_executable(&paths.wokcore_install_record).map_err(|error| match error {
            PlatformError::InvalidWokCoreInstallRecord => CoreOperationError::InvalidInstallState,
            _ => CoreOperationError::Initialization,
        })
    }
}

struct CoordinatorState {
    active: Option<ActiveOperation>,
    last_snapshot: Option<CoreOperationSnapshot>,
    update_check: Option<CoreUpdateCheck>,
}

struct ActiveOperation {
    operation_id: Uuid,
    operation: CoreOperationKind,
}

#[derive(Clone)]
pub(crate) struct CoreOperationCoordinator {
    runtime: Arc<DesktopRuntimeState>,
    state: Arc<Mutex<CoordinatorState>>,
    update_check_gate: Arc<Mutex<()>>,
    runner: Arc<dyn OperationRunner>,
    authority: Arc<dyn TrustedRuntimeAuthority>,
    persistent_operations: bool,
    journal: Option<Arc<OperationJournal>>,
    helper_launcher: Option<Arc<dyn HelperLauncher>>,
    recovery_probe: Option<Arc<dyn OperationRecoveryProbe>>,
}

impl CoreOperationCoordinator {
    pub(crate) fn new(runtime: Arc<DesktopRuntimeState>) -> Self {
        let paths = AppPaths::discover().ok();
        let journal = paths
            .as_ref()
            .and_then(|paths| OperationJournal::open(&paths.runtime_dir).ok())
            .map(Arc::new);
        let recovery_probe = paths.as_ref().map(|paths| {
            Arc::new(SystemOperationRecoveryProbe {
                runtime: runtime.clone(),
                install_directory: paths.wokcore_install_dir.clone(),
            }) as Arc<dyn OperationRecoveryProbe>
        });
        Self {
            runtime,
            state: Arc::new(Mutex::new(CoordinatorState {
                active: None,
                last_snapshot: None,
                update_check: None,
            })),
            update_check_gate: Arc::new(Mutex::new(())),
            runner: Arc::new(SystemOperationRunner),
            authority: Arc::new(SystemTrustedRuntimeAuthority::discover()),
            persistent_operations: true,
            journal,
            helper_launcher: Some(Arc::new(SystemHelperLauncher)),
            recovery_probe,
        }
    }

    #[cfg(test)]
    fn new_with_dependencies(
        runtime: Arc<DesktopRuntimeState>,
        runner: Arc<dyn OperationRunner>,
        authority: Arc<dyn TrustedRuntimeAuthority>,
    ) -> Self {
        Self {
            runtime,
            state: Arc::new(Mutex::new(CoordinatorState {
                active: None,
                last_snapshot: None,
                update_check: None,
            })),
            update_check_gate: Arc::new(Mutex::new(())),
            runner,
            authority,
            persistent_operations: false,
            journal: None,
            helper_launcher: None,
            recovery_probe: None,
        }
    }

    #[cfg(test)]
    fn new_persistent_with_dependencies(
        runtime: Arc<DesktopRuntimeState>,
        runner: Arc<dyn OperationRunner>,
        authority: Arc<dyn TrustedRuntimeAuthority>,
        journal: Arc<OperationJournal>,
        helper_launcher: Arc<dyn HelperLauncher>,
        recovery_probe: Arc<dyn OperationRecoveryProbe>,
    ) -> Self {
        Self {
            runtime,
            state: Arc::new(Mutex::new(CoordinatorState {
                active: None,
                last_snapshot: None,
                update_check: None,
            })),
            update_check_gate: Arc::new(Mutex::new(())),
            runner,
            authority,
            persistent_operations: true,
            journal: Some(journal),
            helper_launcher: Some(helper_launcher),
            recovery_probe: Some(recovery_probe),
        }
    }

    #[cfg(test)]
    pub(crate) async fn status(&self) -> Option<CoreOperationSnapshot> {
        self.status_result().await.ok().flatten()
    }

    pub(crate) async fn status_with_sink(
        &self,
        sink: Arc<dyn OperationEventSink>,
    ) -> Result<Option<CoreOperationSnapshot>, CoreOperationError> {
        let snapshot = self.status_result().await?;
        if let Some(snapshot) = &snapshot
            && snapshot.state == CoreOperationState::Running
            && self.persistent_operations
        {
            self.ensure_persistent_monitor(snapshot.clone(), sink).await;
        }
        Ok(snapshot)
    }

    async fn status_result(&self) -> Result<Option<CoreOperationSnapshot>, CoreOperationError> {
        if self.persistent_operations {
            return self.persistent_status().await;
        }
        Ok(self.state.lock().await.last_snapshot.clone())
    }

    pub(crate) async fn install_and_start(
        &self,
        sink: Arc<dyn OperationEventSink>,
    ) -> Result<CoreOperationSnapshot, CoreOperationError> {
        self.require_production_channel().await?;
        self.start_operation(CoreOperationKind::Install, OperationRequest::Install, sink)
            .await
    }

    pub(crate) async fn check_update(&self) -> Result<CoreUpdateCheck, CoreOperationError> {
        let _gate = self.update_check_gate.lock().await;
        if let Some(check) = self.state.lock().await.update_check.clone() {
            return Ok(check);
        }
        if self.operation_is_active().await? {
            return Err(CoreOperationError::OperationInProgress);
        }
        let executable = self.trusted_production_executable().await?;
        let completion = self
            .runner
            .clone()
            .check_update(executable)
            .await
            .map_err(|_| CoreOperationError::UpdateVerificationFailed)?;
        let check = parse_update_check(completion)?;
        self.state.lock().await.update_check = Some(check.clone());
        Ok(check)
    }

    pub(crate) async fn install_update(
        &self,
        expected_version: &str,
        sink: Arc<dyn OperationEventSink>,
    ) -> Result<CoreOperationSnapshot, CoreOperationError> {
        self.require_production_channel().await?;
        Version::parse(expected_version).map_err(|_| CoreOperationError::InvalidProgress)?;
        if self.operation_is_active().await? {
            return Err(CoreOperationError::OperationInProgress);
        }
        let executable = self.trusted_production_executable().await?;
        self.start_operation(
            CoreOperationKind::Update,
            OperationRequest::Update { executable },
            sink,
        )
        .await
    }

    async fn require_production_channel(&self) -> Result<(), CoreOperationError> {
        let runtime = self
            .runtime
            .selected()
            .await
            .map_err(|_| CoreOperationError::Initialization)?;
        if runtime.channel() == WokCoreRuntimeChannel::Development {
            return Err(CoreOperationError::DevelopmentRuntimeManagedByIde);
        }
        Ok(())
    }

    async fn trusted_production_executable(&self) -> Result<PathBuf, CoreOperationError> {
        let runtime = self
            .runtime
            .selected()
            .await
            .map_err(|_| CoreOperationError::Initialization)?;
        if runtime.channel() == WokCoreRuntimeChannel::Development {
            return Err(CoreOperationError::DevelopmentRuntimeManagedByIde);
        }
        if let Some(executable) = runtime.executable() {
            return Ok(executable.to_path_buf());
        }
        self.authority
            .discover()?
            .ok_or(CoreOperationError::UpdateUnavailable)
    }

    async fn start_operation(
        &self,
        operation: CoreOperationKind,
        request: OperationRequest,
        sink: Arc<dyn OperationEventSink>,
    ) -> Result<CoreOperationSnapshot, CoreOperationError> {
        if self.persistent_operations {
            return self.start_persistent_operation(operation, sink).await;
        }
        self.start_legacy_operation(operation, request, sink).await
    }

    async fn operation_is_active(&self) -> Result<bool, CoreOperationError> {
        if self.persistent_operations {
            return Ok(self.persistent_status().await?.is_some_and(|snapshot| {
                snapshot.state == CoreOperationState::Running
                    || (snapshot.operation == CoreOperationKind::Install
                        && snapshot.state == CoreOperationState::Failed
                        && snapshot.error_code.as_deref() == Some("install_in_progress"))
            }));
        }
        Ok(self.state.lock().await.active.is_some())
    }

    async fn start_legacy_operation(
        &self,
        operation: CoreOperationKind,
        request: OperationRequest,
        sink: Arc<dyn OperationEventSink>,
    ) -> Result<CoreOperationSnapshot, CoreOperationError> {
        let snapshot = {
            let mut state = self.state.lock().await;
            if let Some(active) = &state.active {
                if operation == CoreOperationKind::Install
                    && active.operation == CoreOperationKind::Install
                {
                    return state
                        .last_snapshot
                        .clone()
                        .ok_or(CoreOperationError::InvalidProgress);
                }
                return Err(CoreOperationError::OperationInProgress);
            }
            let snapshot = CoreOperationSnapshot::initial(operation);
            state.active = Some(ActiveOperation {
                operation_id: snapshot.operation_id,
                operation,
            });
            state.last_snapshot = Some(snapshot.clone());
            snapshot
        };
        sink.emit(&snapshot).await;
        let operation_id = snapshot.operation_id;
        let operation = snapshot.operation;
        let (sender, receiver) = mpsc::channel(32);
        let runner = tauri::async_runtime::spawn(self.runner.clone().run(request, sender));
        let coordinator = self.clone();
        tauri::async_runtime::spawn(async move {
            coordinator
                .run_operation(operation_id, operation, receiver, runner, sink)
                .await;
        });
        Ok(snapshot)
    }

    async fn start_persistent_operation(
        &self,
        operation: CoreOperationKind,
        sink: Arc<dyn OperationEventSink>,
    ) -> Result<CoreOperationSnapshot, CoreOperationError> {
        let journal = self
            .journal
            .as_ref()
            .ok_or(CoreOperationError::Initialization)?;
        let launcher = self
            .helper_launcher
            .as_ref()
            .ok_or(CoreOperationError::Initialization)?;
        let claim = self.acquire_claim(journal).await?;
        if let Some(existing) = journal.read()? {
            let existing = self.reconcile_locked(journal, existing).await?;
            if existing.state == CoreOperationState::Running {
                drop(claim);
                self.store_persistent_snapshot(existing.clone()).await;
                if operation == CoreOperationKind::Install
                    && existing.operation == CoreOperationKind::Install
                {
                    self.ensure_persistent_monitor(existing.clone(), sink).await;
                    return Ok(existing);
                }
                return Err(CoreOperationError::OperationInProgress);
            }
            if existing.operation == CoreOperationKind::Install
                && existing.state == CoreOperationState::Failed
                && existing.error_code.as_deref() == Some("install_in_progress")
            {
                drop(claim);
                self.store_persistent_snapshot(existing.clone()).await;
                if operation == CoreOperationKind::Install {
                    return Ok(existing);
                }
                return Err(CoreOperationError::OperationInProgress);
            }
        }

        let snapshot = CoreOperationSnapshot::initial(operation);
        journal.write(&snapshot)?;
        self.store_persistent_snapshot(snapshot.clone()).await;
        let mut helper = match launcher.launch(snapshot.operation_id, operation) {
            Ok(helper) => helper,
            Err(_) => {
                let failed = CoreOperationSnapshot::failed(
                    snapshot.operation_id,
                    1,
                    operation,
                    operation_failure_code(operation),
                );
                journal.write(&failed)?;
                drop(claim);
                self.store_persistent_snapshot(failed.clone()).await;
                sink.emit(&failed).await;
                return Ok(failed);
            }
        };

        let ready = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let current = journal.read()?.ok_or(CoreOperationError::InvalidProgress)?;
                if current.operation_id != snapshot.operation_id || current.operation != operation {
                    return Err(CoreOperationError::InvalidProgress);
                }
                if current.state != CoreOperationState::Running
                    || journal.operation_lease_active()?
                {
                    return Ok(current);
                }
                if helper
                    .try_wait()
                    .map_err(|_| CoreOperationError::Initialization)?
                    .is_some()
                {
                    return self.reconcile_locked(journal, current).await;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        let ready = match ready {
            Ok(Ok(ready)) => ready,
            Ok(Err(error)) => {
                drop(claim);
                reap_helper(helper);
                return Err(error);
            }
            Err(_) => {
                let resolution = match fence_helper_timeout(journal, &snapshot) {
                    Ok(resolution) => resolution,
                    Err(error) => {
                        drop(claim);
                        reap_helper(helper);
                        return Err(error);
                    }
                };
                match resolution {
                    HelperTimeoutResolution::Active(current) => current,
                    HelperTimeoutResolution::Fenced(fence) => {
                        let _ = helper.kill().await;
                        let _ = helper.wait().await;
                        let failed = CoreOperationSnapshot::failed(
                            snapshot.operation_id,
                            snapshot.sequence + 1,
                            operation,
                            operation_failure_code(operation),
                        );
                        let write_result = journal.write(&failed);
                        drop(fence);
                        write_result?;
                        drop(claim);
                        self.store_persistent_snapshot(failed.clone()).await;
                        sink.emit(&failed).await;
                        return Ok(failed);
                    }
                }
            }
        };
        drop(claim);
        reap_helper(helper);
        self.store_persistent_snapshot(ready.clone()).await;
        sink.emit(&ready).await;
        if ready.state == CoreOperationState::Running {
            self.ensure_persistent_monitor(ready.clone(), sink).await;
        }
        Ok(ready)
    }

    async fn persistent_status(&self) -> Result<Option<CoreOperationSnapshot>, CoreOperationError> {
        let journal = self
            .journal
            .as_ref()
            .ok_or(CoreOperationError::Initialization)?;
        let Some(snapshot) = journal.read()? else {
            return Ok(None);
        };
        let needs_recovery = (snapshot.state == CoreOperationState::Running
            && !journal.operation_lease_active()?)
            || (snapshot.operation == CoreOperationKind::Install
                && snapshot.state == CoreOperationState::Failed
                && snapshot.error_code.as_deref() == Some("install_in_progress"));
        let snapshot = if needs_recovery {
            let claim = self.acquire_claim(journal).await?;
            let current = journal.read()?.ok_or(CoreOperationError::InvalidProgress)?;
            let current = self.reconcile_locked(journal, current).await?;
            drop(claim);
            current
        } else {
            snapshot
        };
        self.store_persistent_snapshot(snapshot.clone()).await;
        Ok(Some(snapshot))
    }

    async fn reconcile_locked(
        &self,
        journal: &OperationJournal,
        mut snapshot: CoreOperationSnapshot,
    ) -> Result<CoreOperationSnapshot, CoreOperationError> {
        if snapshot.state == CoreOperationState::Running
            && !journal.operation_lease_active()?
            && snapshot.sequence == 0
            && let Some(current) = self.restart_initial_handoff(journal, &snapshot).await?
        {
            snapshot = current;
        }
        if snapshot.state == CoreOperationState::Running && !journal.operation_lease_active()? {
            let recovery_lease = match journal.try_operation_lease()? {
                LeaseAttempt::Busy => {
                    return journal.read()?.ok_or(CoreOperationError::InvalidProgress);
                }
                LeaseAttempt::Acquired(lease) => lease,
            };
            let current = journal.read()?.ok_or(CoreOperationError::InvalidProgress)?;
            if current.operation_id != snapshot.operation_id
                || current.operation != snapshot.operation
            {
                return Err(CoreOperationError::InvalidProgress);
            }
            if current.state != CoreOperationState::Running {
                return Ok(current);
            }
            let recovered = self.recover_from_runtime(&current).await;
            journal.write(&recovered)?;
            drop(recovery_lease);
            return Ok(recovered);
        }
        if snapshot.operation == CoreOperationKind::Install
            && snapshot.state == CoreOperationState::Failed
            && snapshot.error_code.as_deref() == Some("install_in_progress")
        {
            let probe = self
                .recovery_probe
                .as_ref()
                .ok_or(CoreOperationError::Initialization)?;
            if probe.install_lease_active()? {
                return Ok(snapshot);
            }
            let recovered = self.recover_from_runtime(&snapshot).await;
            journal.write(&recovered)?;
            return Ok(recovered);
        }
        Ok(snapshot)
    }

    async fn restart_initial_handoff(
        &self,
        journal: &OperationJournal,
        snapshot: &CoreOperationSnapshot,
    ) -> Result<Option<CoreOperationSnapshot>, CoreOperationError> {
        let launcher = self
            .helper_launcher
            .as_ref()
            .ok_or(CoreOperationError::Initialization)?;
        let mut helper = match launcher.launch(snapshot.operation_id, snapshot.operation) {
            Ok(helper) => helper,
            Err(_) => return Ok(None),
        };
        let observed = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let current = journal.read()?.ok_or(CoreOperationError::InvalidProgress)?;
                if current.operation_id != snapshot.operation_id
                    || current.operation != snapshot.operation
                {
                    return Err(CoreOperationError::InvalidProgress);
                }
                if current.state != CoreOperationState::Running
                    || current.sequence != 0
                    || journal.operation_lease_active()?
                {
                    return Ok(Some(current));
                }
                if helper
                    .try_wait()
                    .map_err(|_| CoreOperationError::Initialization)?
                    .is_some()
                {
                    let current = journal.read()?.ok_or(CoreOperationError::InvalidProgress)?;
                    if current.operation_id != snapshot.operation_id
                        || current.operation != snapshot.operation
                    {
                        return Err(CoreOperationError::InvalidProgress);
                    }
                    if current.state != CoreOperationState::Running
                        || current.sequence != 0
                        || journal.operation_lease_active()?
                    {
                        return Ok(Some(current));
                    }
                    return Ok(None);
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        match observed {
            Ok(result) => {
                reap_helper(helper);
                result
            }
            Err(_) => {
                let resolution = match fence_helper_timeout(journal, snapshot) {
                    Ok(resolution) => resolution,
                    Err(error) => {
                        reap_helper(helper);
                        return Err(error);
                    }
                };
                match resolution {
                    HelperTimeoutResolution::Active(current) => {
                        reap_helper(helper);
                        Ok(Some(current))
                    }
                    HelperTimeoutResolution::Fenced(fence) => {
                        let _ = helper.kill().await;
                        let _ = helper.wait().await;
                        drop(fence);
                        Ok(None)
                    }
                }
            }
        }
    }

    async fn recover_from_runtime(
        &self,
        snapshot: &CoreOperationSnapshot,
    ) -> CoreOperationSnapshot {
        let runtime = match &self.recovery_probe {
            Some(probe) => probe
                .runtime()
                .await
                .unwrap_or(RecoveryRuntimeState::Unavailable),
            None => RecoveryRuntimeState::Unavailable,
        };
        let sequence = snapshot.sequence.saturating_add(1);
        match runtime {
            RecoveryRuntimeState::Ready { version } => {
                let current_version = version.filter(|value| {
                    value.len() <= 64
                        && value.is_ascii()
                        && Version::parse(value).is_ok_and(|parsed| parsed.to_string() == *value)
                });
                let update_matches_target = matches!(
                    (&snapshot.target_version, &current_version),
                    (Some(target), Some(current)) if target == current
                );
                if snapshot.operation == CoreOperationKind::Update && !update_matches_target {
                    return CoreOperationSnapshot::failed(
                        snapshot.operation_id,
                        sequence,
                        snapshot.operation,
                        "update_install_failed",
                    );
                }
                CoreOperationSnapshot {
                    schema_version: 1,
                    operation_id: snapshot.operation_id,
                    sequence,
                    operation: snapshot.operation,
                    state: CoreOperationState::Succeeded,
                    phase: CoreOperationPhase::Completed,
                    current_version,
                    target_version: snapshot.target_version.clone(),
                    bytes_completed: None,
                    bytes_total: None,
                    active_requests: None,
                    error_code: None,
                }
            }
            RecoveryRuntimeState::Missing | RecoveryRuntimeState::Unavailable => {
                CoreOperationSnapshot::failed(
                    snapshot.operation_id,
                    sequence,
                    snapshot.operation,
                    match snapshot.operation {
                        CoreOperationKind::Install => "install_failed",
                        CoreOperationKind::Update => "update_install_failed",
                    },
                )
            }
        }
    }

    async fn acquire_claim(
        &self,
        journal: &OperationJournal,
    ) -> Result<JournalLease, CoreOperationError> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match journal.try_claim()? {
                    LeaseAttempt::Acquired(claim) => return Ok(claim),
                    LeaseAttempt::Busy => tokio::time::sleep(Duration::from_millis(10)).await,
                }
            }
        })
        .await
        .map_err(|_| CoreOperationError::Initialization)?
    }

    async fn store_persistent_snapshot(&self, snapshot: CoreOperationSnapshot) {
        self.state.lock().await.last_snapshot = Some(snapshot);
    }

    async fn ensure_persistent_monitor(
        &self,
        snapshot: CoreOperationSnapshot,
        sink: Arc<dyn OperationEventSink>,
    ) {
        {
            let mut state = self.state.lock().await;
            if state
                .active
                .as_ref()
                .is_some_and(|active| active.operation_id == snapshot.operation_id)
            {
                return;
            }
            state.active = Some(ActiveOperation {
                operation_id: snapshot.operation_id,
                operation: snapshot.operation,
            });
        }
        let coordinator = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut sequence = snapshot.sequence;
            loop {
                tokio::time::sleep(Duration::from_millis(25)).await;
                let Ok(Some(current)) = coordinator.persistent_status().await else {
                    break;
                };
                if current.operation_id != snapshot.operation_id {
                    break;
                }
                if current.sequence > sequence {
                    sequence = current.sequence;
                    sink.emit(&current).await;
                }
                if current.state != CoreOperationState::Running {
                    break;
                }
            }
            let mut state = coordinator.state.lock().await;
            if state
                .active
                .as_ref()
                .is_some_and(|active| active.operation_id == snapshot.operation_id)
            {
                state.active = None;
            }
        });
    }

    async fn run_operation(
        &self,
        operation_id: Uuid,
        operation: CoreOperationKind,
        mut receiver: mpsc::Receiver<ChildProgress>,
        runner: tauri::async_runtime::JoinHandle<Result<ChildCompletion, RunnerError>>,
        sink: Arc<dyn OperationEventSink>,
    ) {
        let mut terminal = None;
        while let Some(event) = receiver.recv().await {
            if event.state == CoreOperationState::Running {
                self.publish_child(operation_id, event, sink.as_ref()).await;
            } else {
                terminal = Some(event);
            }
        }
        let result = match runner.await {
            Ok(result) => result,
            Err(_) => Err(RunnerError::Wait),
        };
        self.finish_operation(operation_id, operation, terminal, result, sink.as_ref())
            .await;
    }

    async fn publish_child(
        &self,
        operation_id: Uuid,
        child: ChildProgress,
        sink: &dyn OperationEventSink,
    ) {
        let snapshot = {
            let mut state = self.state.lock().await;
            let Some(active) = &state.active else {
                return;
            };
            if active.operation_id != operation_id || active.operation != child.operation {
                return;
            }
            let Some(sequence) = state
                .last_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.sequence.checked_add(1))
            else {
                return;
            };
            let snapshot = CoreOperationSnapshot::from_child(operation_id, sequence, child);
            state.last_snapshot = Some(snapshot.clone());
            snapshot
        };
        sink.emit(&snapshot).await;
    }

    async fn finish_operation(
        &self,
        operation_id: Uuid,
        operation: CoreOperationKind,
        terminal: Option<ChildProgress>,
        result: Result<ChildCompletion, RunnerError>,
        sink: &dyn OperationEventSink,
    ) {
        let snapshot = {
            let mut state = self.state.lock().await;
            let Some(active) = &state.active else {
                return;
            };
            if active.operation_id != operation_id {
                return;
            }
            let sequence = state
                .last_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.sequence.checked_add(1))
                .unwrap_or(u64::MAX);
            let snapshot = final_operation_snapshot(
                operation_id,
                sequence,
                operation,
                terminal,
                &result,
                true,
            );
            state.active = None;
            state.last_snapshot = Some(snapshot.clone());
            snapshot
        };
        sink.emit(&snapshot).await;
    }
}

fn final_operation_snapshot(
    operation_id: Uuid,
    sequence: u64,
    operation: CoreOperationKind,
    terminal: Option<ChildProgress>,
    result: &Result<ChildCompletion, RunnerError>,
    progress_persisted: bool,
) -> CoreOperationSnapshot {
    let runner_failure = result.as_ref().err().copied();
    let final_child = match result {
        Ok(completion)
            if progress_persisted
                && completion.progress_valid
                && completion.last_progress.as_ref() == terminal.as_ref()
                && terminal
                    .as_ref()
                    .is_some_and(|event| event.operation == operation) =>
        {
            let event = terminal.clone().unwrap();
            let terminal_is_accepted = event.state == CoreOperationState::Failed
                || (completion.exit_success
                    && final_stdout_is_valid(operation, &completion.stdout)
                    && event.state == CoreOperationState::Succeeded
                    && event.phase == CoreOperationPhase::Completed);
            terminal_is_accepted.then_some(event)
        }
        _ => None,
    };
    match final_child {
        Some(child) => CoreOperationSnapshot::from_child(operation_id, sequence, child),
        None => {
            let error_code = match runner_failure {
                Some(RunnerError::Spawn | RunnerError::Wait | RunnerError::Read)
                    if progress_persisted =>
                {
                    operation_failure_code(operation)
                }
                _ => "invalid_progress",
            };
            CoreOperationSnapshot::failed(operation_id, sequence, operation, error_code)
        }
    }
}

fn operation_failure_code(operation: CoreOperationKind) -> &'static str {
    match operation {
        CoreOperationKind::Install => "start_failed",
        CoreOperationKind::Update => "update_install_failed",
    }
}

fn final_stdout_is_valid(operation: CoreOperationKind, stdout: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<Value>(stdout) else {
        return false;
    };
    let Some(object) = value.as_object() else {
        return false;
    };
    let Some(code) = object.get("code").and_then(Value::as_str) else {
        return false;
    };
    match (operation, code) {
        (CoreOperationKind::Install, "running" | "already_running") => true,
        (CoreOperationKind::Update, "current") => object
            .get("current_version")
            .and_then(Value::as_str)
            .is_some_and(|version| Version::parse(version).is_ok()),
        (CoreOperationKind::Update, "installed") => ["from", "to"].into_iter().all(|field| {
            object
                .get(field)
                .and_then(Value::as_str)
                .is_some_and(|version| Version::parse(version).is_ok())
        }),
        _ => false,
    }
}

fn parse_update_check(completion: CheckCompletion) -> Result<CoreUpdateCheck, CoreOperationError> {
    let value = serde_json::from_slice::<Value>(&completion.stdout)
        .map_err(|_| CoreOperationError::UpdateVerificationFailed)?;
    let object = value
        .as_object()
        .ok_or(CoreOperationError::UpdateVerificationFailed)?;
    let code = object
        .get("code")
        .and_then(Value::as_str)
        .ok_or(CoreOperationError::UpdateVerificationFailed)?;
    if !completion.exit_success {
        return Err(match code {
            "update_unavailable" => CoreOperationError::UpdateUnavailable,
            "incompatible_manifest" | "update_verification_failed" => {
                CoreOperationError::UpdateVerificationFailed
            }
            _ => CoreOperationError::UpdateVerificationFailed,
        });
    }
    let current_version = object
        .get("current_version")
        .and_then(Value::as_str)
        .filter(|version| Version::parse(version).is_ok())
        .ok_or(CoreOperationError::UpdateVerificationFailed)?
        .to_owned();
    let target_version = match code {
        "current" => None,
        "update_available" => Some(
            object
                .get("version")
                .and_then(Value::as_str)
                .filter(|version| Version::parse(version).is_ok())
                .ok_or(CoreOperationError::UpdateVerificationFailed)?
                .to_owned(),
        ),
        _ => return Err(CoreOperationError::UpdateVerificationFailed),
    };
    Ok(CoreUpdateCheck {
        code: code.to_owned(),
        current_version,
        target_version,
    })
}

struct SystemOperationRunner;

impl OperationRunner for SystemOperationRunner {
    fn run(
        self: Arc<Self>,
        request: OperationRequest,
        progress: mpsc::Sender<ChildProgress>,
    ) -> OperationFuture {
        Box::pin(async move {
            let (operation, spec) = match request {
                OperationRequest::Install => (
                    CoreOperationKind::Install,
                    CommandSpec::install(bundled_wokrouter_executable()?),
                ),
                OperationRequest::Update { executable } => (
                    CoreOperationKind::Update,
                    CommandSpec::update_install(executable),
                ),
            };
            run_progress_child(spec, operation, progress).await
        })
    }

    fn check_update(self: Arc<Self>, executable: PathBuf) -> CheckFuture {
        Box::pin(async move { run_check_child(CommandSpec::update_check(executable)).await })
    }
}

struct CommandSpec {
    program: PathBuf,
    arguments: Vec<String>,
}

impl CommandSpec {
    fn install(program: PathBuf) -> Self {
        Self::raw(program, ["start", "--json", "--progress-jsonl"])
    }

    fn update_check(program: PathBuf) -> Self {
        Self::raw(program, ["update", "--check", "--json"])
    }

    fn update_install(program: PathBuf) -> Self {
        Self::raw(
            program,
            ["update", "--install", "--json", "--progress-jsonl"],
        )
    }

    fn raw(program: PathBuf, arguments: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            program,
            arguments: arguments.into_iter().map(Into::into).collect(),
        }
    }
}

fn bundled_wokrouter_executable() -> Result<PathBuf, RunnerError> {
    let current = env::current_exe().map_err(|_| RunnerError::Spawn)?;
    Ok(current.with_file_name(format!("wokrouter{}", env::consts::EXE_SUFFIX)))
}

async fn run_progress_child(
    spec: CommandSpec,
    operation: CoreOperationKind,
    progress: mpsc::Sender<ChildProgress>,
) -> Result<ChildCompletion, RunnerError> {
    let mut child = spawn_child(&spec)?;
    let stdout = child.stdout.take().ok_or(RunnerError::Spawn)?;
    let stderr = child.stderr.take().ok_or(RunnerError::Spawn)?;
    let (status, stdout, parsed) = tokio::join!(
        child.wait(),
        read_bounded(stdout),
        read_progress(stderr, operation, progress),
    );
    let status = status.map_err(|_| RunnerError::Wait)?;
    let stdout = stdout?;
    let (progress_valid, last_progress) = parsed?;
    Ok(ChildCompletion {
        exit_success: status.success(),
        stdout,
        progress_valid,
        last_progress,
    })
}

async fn run_check_child(spec: CommandSpec) -> Result<CheckCompletion, RunnerError> {
    let mut child = spawn_child(&spec)?;
    let stdout = child.stdout.take().ok_or(RunnerError::Spawn)?;
    let stderr = child.stderr.take().ok_or(RunnerError::Spawn)?;
    let (status, stdout, stderr) =
        tokio::join!(child.wait(), read_bounded(stdout), read_bounded(stderr),);
    let status = status.map_err(|_| RunnerError::Wait)?;
    let stdout = stdout?;
    let _stderr = stderr?;
    Ok(CheckCompletion {
        exit_success: status.success(),
        stdout,
    })
}

fn spawn_child(spec: &CommandSpec) -> Result<tokio::process::Child, RunnerError> {
    let policy = child_process_policy();
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(policy.kill_on_drop);
    #[cfg(windows)]
    command.creation_flags(policy.creation_flags);
    command.spawn().map_err(|_| RunnerError::Spawn)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ChildProcessPolicy {
    kill_on_drop: bool,
    #[cfg(windows)]
    creation_flags: u32,
}

fn child_process_policy() -> ChildProcessPolicy {
    ChildProcessPolicy {
        kill_on_drop: false,
        #[cfg(windows)]
        creation_flags: 0x0800_0000,
    }
}

async fn read_bounded(mut stream: impl AsyncRead + Unpin) -> Result<Vec<u8>, RunnerError> {
    let mut output = Vec::new();
    let mut overflow = false;
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|_| RunnerError::Read)?;
        if read == 0 {
            break;
        }
        if !overflow && output.len().saturating_add(read) <= MAX_BUFFER_BYTES {
            output.extend_from_slice(&chunk[..read]);
        } else {
            overflow = true;
        }
    }
    if overflow {
        Err(RunnerError::InvalidOutput)
    } else {
        Ok(output)
    }
}

async fn read_progress(
    mut stream: impl AsyncRead + Unpin,
    operation: CoreOperationKind,
    progress: mpsc::Sender<ChildProgress>,
) -> Result<(bool, Option<ChildProgress>), RunnerError> {
    let mut parser = ProgressParser::new(operation);
    let mut valid = true;
    let mut last = None;
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|_| RunnerError::Read)?;
        if read == 0 {
            break;
        }
        if valid {
            match parser.push(&chunk[..read]) {
                Ok(events) => {
                    for event in events {
                        last = Some(event.clone());
                        let _ = progress.send(event).await;
                    }
                }
                Err(_) => valid = false,
            }
        }
    }
    if valid && parser.finish().is_err() {
        valid = false;
    }
    Ok((valid, last))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        env,
        ffi::{OsStr, OsString},
        fs::{self, OpenOptions},
        future::Future,
        io::Write,
        path::{Path, PathBuf},
        pin::Pin,
        process::Stdio,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use tempfile::{TempDir, tempdir};
    use tokio::{
        io::AsyncWriteExt,
        sync::{Notify, Semaphore, mpsc},
    };
    use wokrouter_platform::{
        AppPaths, SelectedWokCoreRuntime,
        test_support::{RuntimeSelectorHarness, secure_private_directory, secure_private_file},
    };
    use wokrouter_wokcore_client::{CoreConnection, WokCoreClient};

    use super::{
        CheckCompletion, ChildCompletion, CoreOperationCoordinator, CoreOperationError,
        CoreOperationKind, CoreOperationPhase, CoreOperationSnapshot, CoreOperationState,
        HelperLauncher, OperationEventSink, OperationFuture, OperationRecoveryProbe,
        OperationRequest, OperationRunner, RecoveryFuture, RecoveryRuntimeState, RunnerError,
        SystemOperationRecoveryProbe, TrustedRuntimeAuthority, journal::OperationJournal,
        parser::ChildProgress,
    };
    use crate::runtime::{DesktopRuntimeError, DesktopRuntimeSelector, DesktopRuntimeState};

    #[test]
    fn system_runner_uses_only_the_three_fixed_child_commands() {
        let sidecar = PathBuf::from(r"C:\Program Files\WokRouter\wokrouter.exe");
        let wokcore = PathBuf::from(r"C:\Program Files\WokCore\wokcore.exe");

        let install = super::CommandSpec::install(sidecar.clone());
        let check = super::CommandSpec::update_check(wokcore.clone());
        let update = super::CommandSpec::update_install(wokcore.clone());

        assert_eq!(install.program, sidecar);
        assert_eq!(install.arguments, ["start", "--json", "--progress-jsonl"]);
        assert_eq!(check.program, wokcore);
        assert_eq!(check.arguments, ["update", "--check", "--json"]);
        assert_eq!(
            update.arguments,
            ["update", "--install", "--json", "--progress-jsonl"]
        );
    }

    #[test]
    fn hidden_helper_request_requires_the_exact_safe_argument_shape() {
        let id = "64c09bda-7afd-4e86-8d61-43bc39a8bc51";
        let valid = super::helper::parse_operation_helper_request(
            [super::helper::OPERATION_HELPER_FLAG, id, "install"].map(OsString::from),
        );
        assert!(matches!(
            valid,
            super::helper::OperationHelperRequest::Valid(invocation)
                if invocation.operation_id.to_string() == id
                    && invocation.operation == CoreOperationKind::Install
        ));
        assert!(matches!(
            super::helper::parse_operation_helper_request(["--version"].map(OsString::from)),
            super::helper::OperationHelperRequest::NotRequested
        ));
        for invalid in [
            vec![super::helper::OPERATION_HELPER_FLAG, id],
            vec![super::helper::OPERATION_HELPER_FLAG, id, "install", "extra"],
            vec![
                super::helper::OPERATION_HELPER_FLAG,
                "not-a-uuid",
                "install",
            ],
            vec![
                super::helper::OPERATION_HELPER_FLAG,
                "64C09BDA-7AFD-4E86-8D61-43BC39A8BC51",
                "install",
            ],
            vec![super::helper::OPERATION_HELPER_FLAG, id, "other"],
        ] {
            assert!(matches!(
                super::helper::parse_operation_helper_request(
                    invalid.into_iter().map(OsString::from)
                ),
                super::helper::OperationHelperRequest::Invalid
            ));
        }
    }

    #[tokio::test]
    async fn unavailable_production_journal_fails_closed_before_runner_or_authority() {
        let fixture = RuntimeFixture::new();
        let runtime = fixture.production_runtime(None).await;
        let (runtime, _) = cached_runtime(runtime);
        let runner = Arc::new(FakeRunner::new([]));
        let authority = Arc::new(CountingAuthority::panic());
        let mut coordinator = CoreOperationCoordinator::new_with_dependencies(
            runtime,
            runner.clone(),
            authority.clone(),
        );
        coordinator.persistent_operations = true;
        let sink = Arc::new(RecordingSink::default());

        assert_eq!(
            coordinator
                .status_with_sink(sink.clone())
                .await
                .unwrap_err(),
            CoreOperationError::Initialization
        );
        assert_eq!(
            coordinator
                .install_and_start(sink.clone())
                .await
                .unwrap_err(),
            CoreOperationError::Initialization
        );
        assert_eq!(
            coordinator.check_update().await.unwrap_err(),
            CoreOperationError::Initialization
        );
        assert_eq!(
            coordinator
                .install_update("0.1.23", sink)
                .await
                .unwrap_err(),
            CoreOperationError::Initialization
        );
        assert_eq!(runner.spawn_count(), 0);
        assert_eq!(authority.calls.load(Ordering::SeqCst), 0);
    }

    #[cfg(windows)]
    #[test]
    fn long_child_policy_is_no_window_without_kill_on_drop() {
        let policy = super::child_process_policy();

        assert_eq!(policy.creation_flags, 0x0800_0000);
        assert!(!policy.kill_on_drop);
    }

    #[tokio::test]
    async fn bounded_reader_drains_the_pipe_after_reaching_64_kib() {
        let (mut writer, reader) = tokio::io::duplex(1024);
        let write = tokio::spawn(async move {
            writer
                .write_all(&vec![b'x'; super::MAX_BUFFER_BYTES + 1])
                .await
                .unwrap();
            writer.shutdown().await.unwrap();
        });

        let result = super::read_bounded(reader).await;

        write.await.unwrap();
        assert_eq!(result.unwrap_err(), RunnerError::InvalidOutput);
    }

    #[tokio::test]
    async fn progress_reader_rejects_an_oversized_line_and_still_drains_the_pipe() {
        let (mut writer, reader) = tokio::io::duplex(1024);
        let write = tokio::spawn(async move {
            writer
                .write_all(&vec![b'x'; super::parser::MAX_LINE_BYTES + 1])
                .await
                .unwrap();
            writer.write_all(b"\ntrailing bytes").await.unwrap();
            writer.shutdown().await.unwrap();
        });
        let (sender, mut receiver) = mpsc::channel(4);

        let (valid, last) = super::read_progress(reader, CoreOperationKind::Install, sender)
            .await
            .unwrap();

        write.await.unwrap();
        assert!(!valid);
        assert!(last.is_none());
        assert!(receiver.recv().await.is_none());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn check_child_has_null_stdin_and_captures_both_output_streams() {
        let spec = super::CommandSpec::raw(
            PathBuf::from("powershell.exe"),
            [
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "$inputText=[Console]::In.ReadToEnd(); if($inputText.Length -ne 0){exit 9}; [Console]::Error.Write('bounded stderr'); [Console]::Out.Write('{\"code\":\"current\",\"current_version\":\"0.1.23\"}')",
            ],
        );

        let completion = super::run_check_child(spec).await.unwrap();

        assert!(completion.exit_success);
        assert_eq!(
            completion.stdout,
            br#"{"code":"current","current_version":"0.1.23"}"#
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn malformed_progress_does_not_kill_the_long_child_and_is_reaped() {
        let directory = tempdir().unwrap();
        let marker = directory.path().join("reaped.txt");
        let marker_literal = marker.display().to_string().replace('\'', "''");
        let script = format!(
            "[Console]::Error.Write('not-json`n'); Start-Sleep -Milliseconds 100; Set-Content -LiteralPath '{marker_literal}' -Value 'finished'; [Console]::Out.Write('{{\"code\":\"running\"}}')"
        );
        let spec = super::CommandSpec::raw(
            PathBuf::from("powershell.exe"),
            ["-NoProfile", "-NonInteractive", "-Command", &script],
        );
        let (sender, _receiver) = mpsc::channel(4);

        let completion = super::run_progress_child(spec, CoreOperationKind::Install, sender)
            .await
            .unwrap();

        assert!(completion.exit_success);
        assert!(!completion.progress_valid);
        assert_eq!(fs::read_to_string(marker).unwrap().trim(), "finished");
    }

    #[tokio::test]
    async fn duplicate_installs_coalesce_conflicts_fail_and_terminal_allows_retry() {
        let fixture = RuntimeFixture::new();
        let runtime = fixture.production_runtime(None).await;
        let (runtime, selector_calls) = cached_runtime(runtime);
        let runner = Arc::new(FakeRunner::new([
            RunPlan::blocked(successful_install()),
            RunPlan::immediate(successful_install()),
        ]));
        let coordinator = CoreOperationCoordinator::new_with_dependencies(
            runtime,
            runner.clone(),
            Arc::new(PanicAuthority),
        );
        let sink = Arc::new(RecordingSink::default());

        let first = coordinator.install_and_start(sink.clone()).await.unwrap();
        let duplicate = coordinator.install_and_start(sink.clone()).await.unwrap();
        assert_eq!(first.operation_id, duplicate.operation_id);
        assert_eq!(runner.spawn_count(), 1);

        let conflict = coordinator
            .install_update("0.1.23", sink.clone())
            .await
            .unwrap_err();
        assert_eq!(conflict.code(), "operation_in_progress");
        assert_eq!(runner.spawn_count(), 1);

        runner.release_one();
        let terminal = wait_for_terminal(&coordinator).await;
        assert_eq!(terminal.state, CoreOperationState::Succeeded);
        assert_eq!(terminal.sequence, first.sequence + 1);
        let retry = coordinator.install_and_start(sink).await.unwrap();
        assert_ne!(first.operation_id, retry.operation_id);
        let retry_terminal = wait_for_terminal(&coordinator).await;
        assert_eq!(retry_terminal.state, CoreOperationState::Succeeded);
        assert_eq!(runner.spawn_count(), 2);
        assert_eq!(selector_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn duplicate_updates_conflict_instead_of_coalescing() {
        let fixture = RuntimeFixture::new();
        let executable = fixture.create_file("production/wokcore");
        let runtime = fixture.production_runtime(Some(executable)).await;
        let (runtime, _) = cached_runtime(runtime);
        let runner = Arc::new(FakeRunner::new([RunPlan::blocked(successful_update())]));
        let coordinator = CoreOperationCoordinator::new_with_dependencies(
            runtime,
            runner.clone(),
            Arc::new(PanicAuthority),
        );
        let sink = Arc::new(RecordingSink::default());

        coordinator
            .install_update("0.1.23", sink.clone())
            .await
            .unwrap();
        assert_eq!(
            coordinator
                .install_update("0.1.23", sink)
                .await
                .unwrap_err()
                .code(),
            "operation_in_progress"
        );
        assert_eq!(runner.spawn_count(), 1);
        runner.release_one();
        assert_eq!(
            wait_for_terminal(&coordinator).await.state,
            CoreOperationState::Succeeded
        );
    }

    #[tokio::test]
    async fn development_suppresses_every_install_and_update_path_before_authority_or_runner() {
        let fixture = RuntimeFixture::new();
        let runtime = fixture.development_runtime().await;
        let (runtime, selector_calls) = cached_runtime(runtime);
        let runner = Arc::new(FakeRunner::new([]));
        let authority = Arc::new(CountingAuthority::panic());
        let coordinator = CoreOperationCoordinator::new_with_dependencies(
            runtime,
            runner.clone(),
            authority.clone(),
        );
        let sink = Arc::new(RecordingSink::default());

        for error in [
            coordinator.check_update().await.unwrap_err(),
            coordinator
                .install_update("0.1.23", sink.clone())
                .await
                .unwrap_err(),
            coordinator
                .install_update("not-semver", sink.clone())
                .await
                .unwrap_err(),
            coordinator.install_and_start(sink).await.unwrap_err(),
        ] {
            assert_eq!(error.code(), "development_runtime_managed_by_ide");
        }
        assert_eq!(authority.calls.load(Ordering::SeqCst), 0);
        assert_eq!(runner.spawn_count(), 0);
        assert_eq!(selector_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn production_uses_the_cached_trusted_executable_without_rediscovery() {
        let fixture = RuntimeFixture::new();
        let executable = fixture.create_file("production/wokcore");
        let runtime = fixture.production_runtime(Some(executable.clone())).await;
        let (runtime, selector_calls) = cached_runtime(runtime);
        let runner = Arc::new(
            FakeRunner::new([RunPlan::immediate(successful_update())]).with_checks([
                Ok(CheckCompletion::success(
                    br#"{"code":"update_available","current_version":"0.1.22","target":"x86_64-pc-windows-msvc","version":"0.1.23"}"#,
                )),
            ]),
        );
        let coordinator = CoreOperationCoordinator::new_with_dependencies(
            runtime,
            runner.clone(),
            Arc::new(PanicAuthority),
        );

        let check = coordinator.check_update().await.unwrap();
        assert_eq!(check.code, "update_available");
        assert_eq!(check.current_version, "0.1.22");
        assert_eq!(check.target_version.as_deref(), Some("0.1.23"));
        coordinator
            .install_update("0.1.23", Arc::new(RecordingSink::default()))
            .await
            .unwrap();
        let terminal = wait_for_terminal(&coordinator).await;
        assert_eq!(terminal.state, CoreOperationState::Succeeded);
        assert_eq!(
            runner.executable_requests(),
            [executable.clone(), executable]
        );
        assert_eq!(selector_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn production_missing_selection_rechecks_only_the_trusted_install_record() {
        let fixture = RuntimeFixture::new();
        let executable = fixture.create_file("installed-later/wokcore");
        let runtime = fixture.production_runtime(None).await;
        let (runtime, selector_calls) = cached_runtime(runtime);
        fixture.write_install_record(&executable);
        let runner = Arc::new(
            FakeRunner::new([RunPlan::immediate(successful_update())]).with_checks([Ok(
                CheckCompletion::success(br#"{"code":"current","current_version":"0.1.23"}"#),
            )]),
        );
        let coordinator = CoreOperationCoordinator::new_with_dependencies(
            runtime,
            runner.clone(),
            Arc::new(super::SystemTrustedRuntimeAuthority::from_paths(
                fixture.paths.clone(),
            )),
        );

        let check = coordinator.check_update().await.unwrap();
        assert_eq!(check.code, "current");
        coordinator
            .install_update("0.1.23", Arc::new(RecordingSink::default()))
            .await
            .unwrap();
        assert_eq!(
            wait_for_terminal(&coordinator).await.state,
            CoreOperationState::Succeeded
        );
        assert_eq!(
            runner.executable_requests(),
            [executable.clone(), executable]
        );
        assert_eq!(selector_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn invalid_trusted_record_and_missing_record_return_stable_codes_without_spawning() {
        for (record, expected) in [
            (
                Some(br#"{"schema_version":1,"executable":"relative"}"#.as_slice()),
                "invalid_install_state",
            ),
            (None, "update_unavailable"),
        ] {
            let fixture = RuntimeFixture::new();
            let runtime = fixture.production_runtime(None).await;
            let (runtime, _) = cached_runtime(runtime);
            if let Some(record) = record {
                fs::write(&fixture.paths.wokcore_install_record, record).unwrap();
                secure_private_file(&fixture.paths.wokcore_install_record).unwrap();
            }
            let runner = Arc::new(FakeRunner::new([]));
            let coordinator = CoreOperationCoordinator::new_with_dependencies(
                runtime,
                runner.clone(),
                Arc::new(super::SystemTrustedRuntimeAuthority::from_paths(
                    fixture.paths.clone(),
                )),
            );

            assert_eq!(
                coordinator.check_update().await.unwrap_err().code(),
                expected
            );
            assert_eq!(runner.spawn_count(), 0);
        }
    }

    #[tokio::test]
    async fn invalid_progress_is_published_only_after_the_runner_is_reaped() {
        let fixture = RuntimeFixture::new();
        let runtime = fixture.production_runtime(None).await;
        let (runtime, _) = cached_runtime(runtime);
        let runner = Arc::new(FakeRunner::new([RunPlan::immediate(Ok(ChildCompletion {
            exit_success: true,
            stdout: br#"{"code":"running"}"#.to_vec(),
            progress_valid: false,
            last_progress: None,
        }))]));
        let coordinator = CoreOperationCoordinator::new_with_dependencies(
            runtime,
            runner.clone(),
            Arc::new(PanicAuthority),
        );
        coordinator
            .install_and_start(Arc::new(RecordingSink::default()))
            .await
            .unwrap();

        let terminal = wait_for_terminal(&coordinator).await;
        assert_eq!(terminal.state, CoreOperationState::Failed);
        assert_eq!(terminal.phase, CoreOperationPhase::Completed);
        assert_eq!(terminal.error_code.as_deref(), Some("invalid_progress"));
        assert_eq!(runner.reaped_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn stable_child_failure_is_preserved_but_runner_failure_is_sanitized() {
        let fixture = RuntimeFixture::new();
        let runtime = fixture.production_runtime(None).await;
        let (runtime, _) = cached_runtime(runtime);
        let runner = Arc::new(FakeRunner::new([
            RunPlan::immediate(ChildCompletion::with_progress(
                false,
                br#"{"code":"start_failed"}"#,
                [progress(
                    r#"{"schema_version":1,"sequence":0,"operation":"install","state":"failed","phase":"starting","error_code":"start_failed"}"#,
                )],
            )),
            RunPlan::immediate(Err(RunnerError::Spawn)),
        ]));
        let coordinator = CoreOperationCoordinator::new_with_dependencies(
            runtime,
            runner,
            Arc::new(PanicAuthority),
        );

        coordinator
            .install_and_start(Arc::new(RecordingSink::default()))
            .await
            .unwrap();
        assert_eq!(
            wait_for_terminal(&coordinator).await.error_code.as_deref(),
            Some("start_failed")
        );
        coordinator
            .install_and_start(Arc::new(RecordingSink::default()))
            .await
            .unwrap();
        let failure = wait_for_terminal(&coordinator).await;
        assert_eq!(failure.error_code.as_deref(), Some("start_failed"));
        assert!(
            !serde_json::to_string(&failure)
                .unwrap()
                .contains(['\\', '/'])
        );
    }

    #[tokio::test]
    async fn snapshots_are_stored_before_each_event_is_emitted() {
        let fixture = RuntimeFixture::new();
        let runtime = fixture.production_runtime(None).await;
        let (runtime, _) = cached_runtime(runtime);
        let runner = Arc::new(FakeRunner::new([RunPlan::blocked(successful_install())]));
        let coordinator = CoreOperationCoordinator::new_with_dependencies(
            runtime,
            runner.clone(),
            Arc::new(PanicAuthority),
        );
        let sink = Arc::new(BlockingSink::default());
        let accepted = coordinator.install_and_start(sink.clone()).await.unwrap();
        assert_eq!(accepted.sequence, 0);

        let terminal_entered = sink.terminal_entered.notified();
        let terminal_completed = sink.terminal_completed.notified();
        runner.release_one();
        terminal_entered.await;

        let stored_during_emit = coordinator.status().await.unwrap();
        assert_eq!(stored_during_emit.sequence, 1);
        assert_eq!(stored_during_emit.state, CoreOperationState::Succeeded);
        assert_eq!(runner.reaped_count.load(Ordering::SeqCst), 1);
        assert!(!sink.finished.load(Ordering::SeqCst));

        sink.release.notify_one();
        tokio::time::timeout(Duration::from_secs(2), terminal_completed)
            .await
            .unwrap();
        assert!(sink.finished.load(Ordering::SeqCst));
        assert_eq!(coordinator.status().await, Some(stored_during_emit));
    }

    #[test]
    fn operation_journal_round_trips_only_the_strict_safe_snapshot() {
        let fixture = RuntimeFixture::new();
        let journal = super::journal::OperationJournal::open(&fixture.paths.runtime_dir).unwrap();
        let snapshot = CoreOperationSnapshot {
            schema_version: 1,
            operation_id: uuid::Uuid::parse_str("64c09bda-7afd-4e86-8d61-43bc39a8bc51").unwrap(),
            sequence: 3,
            operation: CoreOperationKind::Install,
            state: CoreOperationState::Running,
            phase: CoreOperationPhase::Downloading,
            current_version: None,
            target_version: Some("0.1.23".to_owned()),
            bytes_completed: Some(512),
            bytes_total: Some(1024),
            active_requests: None,
            error_code: None,
        };

        journal.write(&snapshot).unwrap();
        assert_eq!(journal.read().unwrap(), Some(snapshot));

        let record_path = fixture
            .paths
            .runtime_dir
            .join("core-operation")
            .join("operation.json");
        let encoded = fs::read_to_string(&record_path).unwrap();
        for forbidden in ["pid", "path", "token", "stderr", "executable"] {
            assert!(!encoded.contains(forbidden), "journal leaked {forbidden}");
        }

        let mut unknown = serde_json::from_str::<serde_json::Value>(&encoded).unwrap();
        unknown
            .as_object_mut()
            .unwrap()
            .insert("pid".to_owned(), serde_json::json!(42));
        fs::write(&record_path, serde_json::to_vec(&unknown).unwrap()).unwrap();
        secure_private_file(&record_path).unwrap();
        assert_eq!(
            journal.read().unwrap_err(),
            CoreOperationError::InvalidProgress
        );

        for (field, value) in [
            (
                "operation_id",
                serde_json::json!("64C09BDA-7AFD-4E86-8D61-43BC39A8BC51"),
            ),
            ("target_version", serde_json::json!("not-semver")),
            ("error_code", serde_json::json!("backend said C:\\secret")),
            ("bytes_completed", serde_json::json!(2048)),
            ("active_requests", serde_json::json!(1)),
        ] {
            let mut invalid = serde_json::from_str::<serde_json::Value>(&encoded).unwrap();
            invalid
                .as_object_mut()
                .unwrap()
                .insert(field.to_owned(), value);
            fs::write(&record_path, serde_json::to_vec(&invalid).unwrap()).unwrap();
            secure_private_file(&record_path).unwrap();
            assert_eq!(
                journal.read().unwrap_err(),
                CoreOperationError::InvalidProgress,
                "journal accepted unsafe field {field}"
            );
        }
    }

    #[test]
    fn helper_timeout_attaches_when_the_bound_operation_lease_is_owned() {
        let fixture = RuntimeFixture::new();
        let journal = OperationJournal::open(&fixture.paths.runtime_dir).unwrap();
        let snapshot = CoreOperationSnapshot::initial(CoreOperationKind::Install);
        journal.write(&snapshot).unwrap();
        let owner = match journal.try_operation_lease().unwrap() {
            super::journal::LeaseAttempt::Acquired(owner) => owner,
            super::journal::LeaseAttempt::Busy => panic!("operation lease should be free"),
        };

        let resolution = super::fence_helper_timeout(&journal, &snapshot).unwrap();

        assert!(matches!(
            resolution,
            super::HelperTimeoutResolution::Active(current) if current == snapshot
        ));
        assert!(journal.operation_lease_active().unwrap());
        drop(owner);
    }

    #[test]
    fn helper_timeout_fence_blocks_a_late_operation_owner() {
        let fixture = RuntimeFixture::new();
        let journal = OperationJournal::open(&fixture.paths.runtime_dir).unwrap();
        let snapshot = CoreOperationSnapshot::initial(CoreOperationKind::Update);
        journal.write(&snapshot).unwrap();

        let fence = match super::fence_helper_timeout(&journal, &snapshot).unwrap() {
            super::HelperTimeoutResolution::Fenced(fence) => fence,
            super::HelperTimeoutResolution::Active(_) => {
                panic!("operation lease should be free")
            }
        };
        assert!(matches!(
            journal.try_operation_lease().unwrap(),
            super::journal::LeaseAttempt::Busy
        ));

        drop(fence);
        assert!(matches!(
            journal.try_operation_lease().unwrap(),
            super::journal::LeaseAttempt::Acquired(_)
        ));
    }

    #[test]
    fn helper_timeout_fence_preserves_a_terminal_written_before_fencing() {
        let fixture = RuntimeFixture::new();
        let journal = OperationJournal::open(&fixture.paths.runtime_dir).unwrap();
        let initial = CoreOperationSnapshot::initial(CoreOperationKind::Install);
        let terminal = CoreOperationSnapshot {
            sequence: 1,
            state: CoreOperationState::Succeeded,
            phase: CoreOperationPhase::Completed,
            ..initial.clone()
        };
        journal.write(&terminal).unwrap();

        let resolution = super::fence_helper_timeout(&journal, &initial).unwrap();

        assert!(matches!(
            resolution,
            super::HelperTimeoutResolution::Active(current) if current == terminal
        ));
    }

    #[tokio::test]
    async fn real_helper_survives_coordinator_reopen_and_prevents_a_second_launch() {
        let fixture = RuntimeFixture::new();
        let marker = fixture.root.path().join("helper-launches.txt");
        let first = persistent_coordinator(
            &fixture,
            ProcessHelperLauncher::new(&fixture, &marker, HelperScenario::InstallSuccess),
            Arc::new(FixedRecoveryProbe::missing(false)),
            None,
        )
        .await;
        let sink = Arc::new(RecordingSink::default());

        let accepted = first.install_and_start(sink).await.unwrap();
        let progress = wait_for_running_sequence(&first, 1).await;
        assert_eq!(progress.operation_id, accepted.operation_id);
        drop(first);

        let reopened_sink = Arc::new(RecordingSink::default());
        let reopened = persistent_coordinator(
            &fixture,
            ProcessHelperLauncher::new(&fixture, &marker, HelperScenario::InstallSuccess),
            Arc::new(FixedRecoveryProbe::missing(false)),
            None,
        )
        .await;
        let recovered = reopened
            .status_with_sink(reopened_sink.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recovered.operation_id, accepted.operation_id);
        assert!(recovered.sequence >= progress.sequence);

        let duplicate = reopened
            .install_and_start(reopened_sink.clone())
            .await
            .unwrap();
        assert_eq!(duplicate.operation_id, accepted.operation_id);
        assert_eq!(launch_count(&marker), 1);

        let terminal = wait_for_terminal(&reopened).await;
        assert_eq!(terminal.operation_id, accepted.operation_id);
        assert_eq!(
            terminal.state,
            CoreOperationState::Succeeded,
            "unexpected terminal: {terminal:?}"
        );
        assert_eq!(launch_count(&marker), 1);
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if reopened_sink
                    .snapshots
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|snapshot| snapshot.state == CoreOperationState::Succeeded)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn reopened_coordinator_recovers_a_real_helper_failure() {
        let fixture = RuntimeFixture::new();
        let marker = fixture.root.path().join("helper-launches.txt");
        let first = persistent_coordinator(
            &fixture,
            ProcessHelperLauncher::new(&fixture, &marker, HelperScenario::InstallFailure),
            Arc::new(FixedRecoveryProbe::missing(false)),
            None,
        )
        .await;
        let accepted = first
            .install_and_start(Arc::new(RecordingSink::default()))
            .await
            .unwrap();
        wait_for_running_sequence(&first, 1).await;
        drop(first);

        let reopened = persistent_coordinator(
            &fixture,
            ProcessHelperLauncher::new(&fixture, &marker, HelperScenario::InstallFailure),
            Arc::new(FixedRecoveryProbe::missing(false)),
            None,
        )
        .await;
        let terminal = wait_for_terminal(&reopened).await;
        assert_eq!(terminal.operation_id, accepted.operation_id);
        assert_eq!(terminal.state, CoreOperationState::Failed);
        assert_eq!(terminal.error_code.as_deref(), Some("start_failed"));
        assert_eq!(launch_count(&marker), 1);
    }

    #[tokio::test]
    async fn released_external_lease_with_missing_core_becomes_install_failed_and_retryable() {
        let fixture = RuntimeFixture::new();
        let marker = fixture.root.path().join("helper-launches.txt");
        let recovery = Arc::new(FixedRecoveryProbe::missing(true));
        let first = persistent_coordinator(
            &fixture,
            ProcessHelperLauncher::new(&fixture, &marker, HelperScenario::InstallConflict),
            recovery.clone(),
            None,
        )
        .await;
        first
            .install_and_start(Arc::new(RecordingSink::default()))
            .await
            .unwrap();
        let conflict = wait_for_terminal(&first).await;
        assert_eq!(conflict.error_code.as_deref(), Some("install_in_progress"));

        recovery.install_lease_active.store(false, Ordering::SeqCst);
        let reopened = persistent_coordinator(
            &fixture,
            ProcessHelperLauncher::new(&fixture, &marker, HelperScenario::InstallSuccess),
            recovery,
            None,
        )
        .await;
        let released = reopened.status().await.unwrap();
        assert_eq!(released.state, CoreOperationState::Failed);
        assert_eq!(released.phase, CoreOperationPhase::Completed);
        assert_eq!(released.error_code.as_deref(), Some("install_failed"));

        let retry = reopened
            .install_and_start(Arc::new(RecordingSink::default()))
            .await
            .unwrap();
        assert_ne!(retry.operation_id, released.operation_id);
        let retry_terminal = wait_for_terminal(&reopened).await;
        assert_eq!(
            retry_terminal.state,
            CoreOperationState::Succeeded,
            "unexpected retry terminal: {retry_terminal:?}"
        );
        assert_eq!(launch_count(&marker), 2);
    }

    #[tokio::test]
    async fn active_external_install_lease_blocks_update_checks_and_installs() {
        let fixture = RuntimeFixture::new();
        let marker = fixture.root.path().join("helper-launches.txt");
        let coordinator = persistent_coordinator(
            &fixture,
            ProcessHelperLauncher::new(&fixture, &marker, HelperScenario::InstallConflict),
            Arc::new(FixedRecoveryProbe::missing(true)),
            None,
        )
        .await;
        coordinator
            .install_and_start(Arc::new(RecordingSink::default()))
            .await
            .unwrap();
        let conflict = wait_for_terminal(&coordinator).await;
        assert_eq!(conflict.error_code.as_deref(), Some("install_in_progress"));

        assert_eq!(
            coordinator.check_update().await.unwrap_err(),
            CoreOperationError::OperationInProgress
        );
        assert_eq!(
            coordinator
                .install_update("0.1.23", Arc::new(RecordingSink::default()))
                .await
                .unwrap_err(),
            CoreOperationError::OperationInProgress
        );
        assert_eq!(launch_count(&marker), 1);
    }

    #[tokio::test]
    async fn real_external_install_lease_survives_one_process_and_releases_for_recovery() {
        let fixture = RuntimeFixture::new();
        fs::create_dir_all(&fixture.paths.wokcore_install_dir).unwrap();
        secure_private_directory(&fixture.paths.wokcore_install_dir).unwrap();
        let lease_ready = fixture.root.path().join("install-lease-ready");
        let lease_release = fixture.root.path().join("install-lease-release");
        let mut holder = tokio::process::Command::new(env::current_exe().unwrap());
        holder
            .args([
                "--exact",
                "core_operation::tests::install_lease_holder_process_entry",
                "--nocapture",
            ])
            .env(
                "WOKROUTER_TEST_INSTALL_LEASE_DIRECTORY",
                &fixture.paths.wokcore_install_dir,
            )
            .env("WOKROUTER_TEST_INSTALL_LEASE_READY", &lease_ready)
            .env("WOKROUTER_TEST_INSTALL_LEASE_RELEASE", &lease_release)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(false);
        #[cfg(windows)]
        holder.creation_flags(0x0800_0000);
        let mut holder = holder.spawn().unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while !lease_ready.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(
            wokrouter_platform::wokcore_install_lease_active(&fixture.paths.wokcore_install_dir)
                .unwrap()
        );

        let marker = fixture.root.path().join("helper-launches.txt");
        let first = persistent_coordinator(
            &fixture,
            ProcessHelperLauncher::new(&fixture, &marker, HelperScenario::InstallConflict),
            system_recovery_probe(&fixture).await,
            None,
        )
        .await;
        first
            .install_and_start(Arc::new(RecordingSink::default()))
            .await
            .unwrap();
        let conflict = wait_for_terminal(&first).await;
        assert_eq!(conflict.error_code.as_deref(), Some("install_in_progress"));
        let held = first.status().await.unwrap();
        assert_eq!(held.error_code.as_deref(), Some("install_in_progress"));

        fs::write(&lease_release, b"release").unwrap();
        assert!(holder.wait().await.unwrap().success());
        assert!(
            !wokrouter_platform::wokcore_install_lease_active(&fixture.paths.wokcore_install_dir)
                .unwrap()
        );

        let reopened = persistent_coordinator(
            &fixture,
            ProcessHelperLauncher::new(&fixture, &marker, HelperScenario::InstallSuccess),
            system_recovery_probe(&fixture).await,
            None,
        )
        .await;
        let released = reopened.status().await.unwrap();
        assert_eq!(released.error_code.as_deref(), Some("install_failed"));
        let retry = reopened
            .install_and_start(Arc::new(RecordingSink::default()))
            .await
            .unwrap();
        assert_ne!(retry.operation_id, released.operation_id);
        assert_eq!(
            wait_for_terminal(&reopened).await.state,
            CoreOperationState::Succeeded
        );
    }

    #[tokio::test]
    async fn reopened_coordinator_recovers_a_real_update_terminal() {
        let fixture = RuntimeFixture::new();
        let executable = fixture.create_file("production/wokcore");
        fixture.write_install_record(&executable);
        let marker = fixture.root.path().join("helper-launches.txt");
        let first = persistent_coordinator(
            &fixture,
            ProcessHelperLauncher::new(&fixture, &marker, HelperScenario::UpdateSuccess),
            Arc::new(FixedRecoveryProbe::missing(false)),
            Some(executable.clone()),
        )
        .await;
        let accepted = first
            .install_update("0.1.23", Arc::new(RecordingSink::default()))
            .await
            .unwrap();
        wait_for_running_sequence(&first, 1).await;
        drop(first);

        let reopened = persistent_coordinator(
            &fixture,
            ProcessHelperLauncher::new(&fixture, &marker, HelperScenario::UpdateSuccess),
            Arc::new(FixedRecoveryProbe::missing(false)),
            Some(executable),
        )
        .await;
        let terminal = wait_for_terminal(&reopened).await;
        assert_eq!(terminal.operation_id, accepted.operation_id);
        assert_eq!(terminal.operation, CoreOperationKind::Update);
        assert_eq!(
            terminal.state,
            CoreOperationState::Succeeded,
            "unexpected update terminal: {terminal:?}"
        );
        assert_eq!(terminal.target_version.as_deref(), Some("0.1.23"));
        assert_eq!(launch_count(&marker), 1);
    }

    #[tokio::test]
    async fn reopened_coordinator_restarts_an_unready_handoff_without_duplicate_runner_work() {
        let fixture = RuntimeFixture::new();
        let launch_marker = fixture.root.path().join("helper-launches.txt");
        let runner_marker = fixture.root.path().join("runner-starts.txt");
        let operation_id = uuid::Uuid::new_v4();
        let mut parent = tokio::process::Command::new(env::current_exe().unwrap());
        parent
            .args([
                "--exact",
                "core_operation::tests::operation_parent_process_entry",
                "--nocapture",
            ])
            .env("WOKROUTER_TEST_PARENT_RUNTIME", &fixture.paths.runtime_dir)
            .env(
                "WOKROUTER_TEST_PARENT_OPERATION_ID",
                operation_id.to_string(),
            )
            .env("WOKROUTER_TEST_PARENT_LAUNCH_MARKER", &launch_marker)
            .env("WOKROUTER_TEST_PARENT_RUNNER_MARKER", &runner_marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(false);
        #[cfg(windows)]
        parent.creation_flags(0x0800_0000);
        let status = parent.spawn().unwrap().wait().await.unwrap();
        assert!(status.success());

        let journal = OperationJournal::open(&fixture.paths.runtime_dir).unwrap();
        let initial = journal.read().unwrap().unwrap();
        assert_eq!(initial.operation_id, operation_id);
        assert_eq!(initial.sequence, 0);
        assert!(!journal.operation_lease_active().unwrap());

        let reopened = persistent_coordinator(
            &fixture,
            ProcessHelperLauncher::new(&fixture, &launch_marker, HelperScenario::InstallSuccess)
                .with_runner_marker(&runner_marker),
            Arc::new(FixedRecoveryProbe::missing(false)),
            None,
        )
        .await;
        let recovered = reopened.status().await.unwrap();
        assert_eq!(recovered.operation_id, operation_id);
        assert_eq!(recovered.state, CoreOperationState::Running);

        let terminal = wait_for_terminal(&reopened).await;
        assert_eq!(terminal.operation_id, operation_id);
        assert_eq!(terminal.state, CoreOperationState::Succeeded);
        assert_eq!(launch_count(&launch_marker), 2);
        assert_eq!(launch_count(&runner_marker), 1);
    }

    #[tokio::test]
    async fn update_recovery_rejects_a_running_old_production_version() {
        let fixture = RuntimeFixture::new();
        let marker = fixture.root.path().join("helper-launches.txt");
        let recovery = Arc::new(FixedRecoveryProbe {
            state: RecoveryRuntimeState::Ready {
                version: Some("0.1.22".to_owned()),
            },
            install_lease_active: AtomicBool::new(false),
        });
        let coordinator = persistent_coordinator(
            &fixture,
            ProcessHelperLauncher::new(&fixture, &marker, HelperScenario::UpdateSuccess),
            recovery,
            None,
        )
        .await;
        let mut snapshot = CoreOperationSnapshot::initial(CoreOperationKind::Update);
        snapshot.sequence = 2;
        snapshot.target_version = Some("0.1.23".to_owned());

        let recovered = coordinator.recover_from_runtime(&snapshot).await;

        assert_eq!(recovered.sequence, 3);
        assert_eq!(recovered.state, CoreOperationState::Failed);
        assert_eq!(recovered.phase, CoreOperationPhase::Completed);
        assert_eq!(
            recovered.error_code.as_deref(),
            Some("update_install_failed")
        );
    }

    #[test]
    fn operation_helper_process_entry() {
        let Some(runtime_directory) = env::var_os("WOKROUTER_TEST_OPERATION_RUNTIME") else {
            return;
        };
        if let Some(delay) = env::var("WOKROUTER_TEST_OPERATION_START_DELAY_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
        {
            std::thread::sleep(Duration::from_millis(delay));
        }
        let operation_id = env::var("WOKROUTER_TEST_OPERATION_ID")
            .ok()
            .and_then(|value| uuid::Uuid::parse_str(&value).ok())
            .unwrap();
        let operation = match env::var("WOKROUTER_TEST_OPERATION_KIND").as_deref() {
            Ok("install") => CoreOperationKind::Install,
            Ok("update") => CoreOperationKind::Update,
            _ => panic!("invalid test operation kind"),
        };
        let scenario =
            HelperScenario::parse(&env::var("WOKROUTER_TEST_OPERATION_SCENARIO").unwrap());
        let journal = Arc::new(OperationJournal::open(Path::new(&runtime_directory)).unwrap());
        let request = match operation {
            CoreOperationKind::Install => OperationRequest::Install,
            CoreOperationKind::Update => OperationRequest::Update {
                executable: PathBuf::from("trusted-test-wokcore"),
            },
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let exit = runtime.block_on(super::helper::run_operation_helper_with_request(
            journal,
            operation_id,
            request,
            Arc::new(ProcessFixtureRunner {
                scenario,
                marker: env::var_os("WOKROUTER_TEST_OPERATION_RUNNER_MARKER").map(PathBuf::from),
            }),
        ));
        assert_eq!(exit, 0);
    }

    #[test]
    fn operation_parent_process_entry() {
        let Some(runtime_directory) = env::var_os("WOKROUTER_TEST_PARENT_RUNTIME") else {
            return;
        };
        let operation_id = env::var("WOKROUTER_TEST_PARENT_OPERATION_ID")
            .ok()
            .and_then(|value| uuid::Uuid::parse_str(&value).ok())
            .unwrap();
        let launch_marker =
            PathBuf::from(env::var_os("WOKROUTER_TEST_PARENT_LAUNCH_MARKER").unwrap());
        let runner_marker =
            PathBuf::from(env::var_os("WOKROUTER_TEST_PARENT_RUNNER_MARKER").unwrap());
        let journal = OperationJournal::open(Path::new(&runtime_directory)).unwrap();
        let mut snapshot = CoreOperationSnapshot::initial(CoreOperationKind::Install);
        snapshot.operation_id = operation_id;
        journal.write(&snapshot).unwrap();

        let fixture = ProcessHelperLauncher {
            runtime_directory: PathBuf::from(runtime_directory),
            marker: launch_marker,
            runner_marker: Some(runner_marker),
            startup_delay: Duration::from_millis(750),
            scenario: HelperScenario::InstallSuccess,
        };
        drop(
            fixture
                .launch(operation_id, CoreOperationKind::Install)
                .unwrap(),
        );
    }

    #[test]
    fn install_lease_holder_process_entry() {
        let Some(directory) = env::var_os("WOKROUTER_TEST_INSTALL_LEASE_DIRECTORY") else {
            return;
        };
        let ready = PathBuf::from(env::var_os("WOKROUTER_TEST_INSTALL_LEASE_READY").unwrap());
        let release = PathBuf::from(env::var_os("WOKROUTER_TEST_INSTALL_LEASE_RELEASE").unwrap());
        let _lease =
            wokrouter_platform::test_support::acquire_wokcore_install_lease(Path::new(&directory))
                .unwrap();
        fs::write(ready, b"ready").unwrap();
        while !release.exists() {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    async fn persistent_coordinator(
        fixture: &RuntimeFixture,
        launcher: ProcessHelperLauncher,
        recovery: Arc<dyn OperationRecoveryProbe>,
        executable: Option<PathBuf>,
    ) -> CoreOperationCoordinator {
        let runtime = fixture.production_runtime(executable).await;
        let (runtime, _) = cached_runtime(runtime);
        CoreOperationCoordinator::new_persistent_with_dependencies(
            runtime,
            Arc::new(FakeRunner::new([])),
            Arc::new(PanicAuthority),
            Arc::new(OperationJournal::open(&fixture.paths.runtime_dir).unwrap()),
            Arc::new(launcher),
            recovery,
        )
    }

    async fn system_recovery_probe(fixture: &RuntimeFixture) -> Arc<dyn OperationRecoveryProbe> {
        let runtime = fixture.production_runtime(None).await;
        let (runtime, _) = cached_runtime(runtime);
        Arc::new(SystemOperationRecoveryProbe {
            runtime,
            install_directory: fixture.paths.wokcore_install_dir.clone(),
        })
    }

    async fn wait_for_running_sequence(
        coordinator: &CoreOperationCoordinator,
        minimum_sequence: u64,
    ) -> CoreOperationSnapshot {
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                if let Some(snapshot) = coordinator.status().await
                    && snapshot.state == CoreOperationState::Running
                    && snapshot.sequence >= minimum_sequence
                {
                    return snapshot;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }

    fn launch_count(marker: &Path) -> usize {
        fs::read_to_string(marker)
            .unwrap_or_default()
            .lines()
            .count()
    }

    #[derive(Clone, Copy, Debug)]
    enum HelperScenario {
        InstallSuccess,
        InstallFailure,
        InstallConflict,
        UpdateSuccess,
    }

    impl HelperScenario {
        fn as_str(self) -> &'static str {
            match self {
                Self::InstallSuccess => "install_success",
                Self::InstallFailure => "install_failure",
                Self::InstallConflict => "install_conflict",
                Self::UpdateSuccess => "update_success",
            }
        }

        fn parse(value: &str) -> Self {
            match value {
                "install_success" => Self::InstallSuccess,
                "install_failure" => Self::InstallFailure,
                "install_conflict" => Self::InstallConflict,
                "update_success" => Self::UpdateSuccess,
                _ => panic!("invalid helper scenario"),
            }
        }
    }

    struct ProcessHelperLauncher {
        runtime_directory: PathBuf,
        marker: PathBuf,
        runner_marker: Option<PathBuf>,
        startup_delay: Duration,
        scenario: HelperScenario,
    }

    impl ProcessHelperLauncher {
        fn new(fixture: &RuntimeFixture, marker: &Path, scenario: HelperScenario) -> Self {
            Self {
                runtime_directory: fixture.paths.runtime_dir.clone(),
                marker: marker.to_owned(),
                runner_marker: None,
                startup_delay: Duration::ZERO,
                scenario,
            }
        }

        fn with_runner_marker(mut self, marker: &Path) -> Self {
            self.runner_marker = Some(marker.to_owned());
            self
        }
    }

    impl HelperLauncher for ProcessHelperLauncher {
        fn launch(
            &self,
            operation_id: uuid::Uuid,
            operation: CoreOperationKind,
        ) -> Result<tokio::process::Child, RunnerError> {
            let mut marker = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.marker)
                .map_err(|_| RunnerError::Spawn)?;
            marker
                .write_all(b"launch\n")
                .and_then(|()| marker.sync_all())
                .map_err(|_| RunnerError::Spawn)?;
            let mut command = tokio::process::Command::new(env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "core_operation::tests::operation_helper_process_entry",
                    "--nocapture",
                ])
                .env("WOKROUTER_TEST_OPERATION_RUNTIME", &self.runtime_directory)
                .env("WOKROUTER_TEST_OPERATION_ID", operation_id.to_string())
                .env(
                    "WOKROUTER_TEST_OPERATION_KIND",
                    match operation {
                        CoreOperationKind::Install => "install",
                        CoreOperationKind::Update => "update",
                    },
                )
                .env("WOKROUTER_TEST_OPERATION_SCENARIO", self.scenario.as_str())
                .env(
                    "WOKROUTER_TEST_OPERATION_START_DELAY_MS",
                    self.startup_delay.as_millis().to_string(),
                )
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(false);
            if let Some(marker) = &self.runner_marker {
                command.env("WOKROUTER_TEST_OPERATION_RUNNER_MARKER", marker);
            }
            #[cfg(windows)]
            command.creation_flags(0x0800_0000);
            command.spawn().map_err(|_| RunnerError::Spawn)
        }
    }

    struct FixedRecoveryProbe {
        state: RecoveryRuntimeState,
        install_lease_active: AtomicBool,
    }

    impl FixedRecoveryProbe {
        fn missing(install_lease_active: bool) -> Self {
            Self {
                state: RecoveryRuntimeState::Missing,
                install_lease_active: AtomicBool::new(install_lease_active),
            }
        }
    }

    impl super::OperationRecoveryProbe for FixedRecoveryProbe {
        fn runtime(&self) -> RecoveryFuture<'_> {
            Box::pin(async move { Ok(self.state.clone()) })
        }

        fn install_lease_active(&self) -> Result<bool, CoreOperationError> {
            Ok(self.install_lease_active.load(Ordering::SeqCst))
        }
    }

    struct ProcessFixtureRunner {
        scenario: HelperScenario,
        marker: Option<PathBuf>,
    }

    impl OperationRunner for ProcessFixtureRunner {
        fn run(
            self: Arc<Self>,
            request: OperationRequest,
            progress_sender: mpsc::Sender<ChildProgress>,
        ) -> OperationFuture {
            Box::pin(async move {
                if let Some(marker) = &self.marker {
                    let mut marker = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(marker)
                        .map_err(|_| RunnerError::Spawn)?;
                    marker
                        .write_all(b"run\n")
                        .and_then(|()| marker.sync_all())
                        .map_err(|_| RunnerError::Spawn)?;
                }
                let operation = match request {
                    OperationRequest::Install => CoreOperationKind::Install,
                    OperationRequest::Update { .. } => CoreOperationKind::Update,
                };
                let (running, terminal, stdout, exit_success) = match self.scenario {
                    HelperScenario::InstallSuccess => (
                        r#"{"schema_version":1,"sequence":0,"operation":"install","state":"running","phase":"downloading","target_version":"0.1.23","bytes_completed":512,"bytes_total":1024}"#,
                        r#"{"schema_version":1,"sequence":1,"operation":"install","state":"succeeded","phase":"completed","target_version":"0.1.23"}"#,
                        r#"{"code":"running"}"#,
                        true,
                    ),
                    HelperScenario::InstallFailure => (
                        r#"{"schema_version":1,"sequence":0,"operation":"install","state":"running","phase":"starting"}"#,
                        r#"{"schema_version":1,"sequence":1,"operation":"install","state":"failed","phase":"starting","error_code":"start_failed"}"#,
                        r#"{"code":"start_failed"}"#,
                        false,
                    ),
                    HelperScenario::InstallConflict => (
                        r#"{"schema_version":1,"sequence":0,"operation":"install","state":"running","phase":"installing"}"#,
                        r#"{"schema_version":1,"sequence":1,"operation":"install","state":"failed","phase":"installing","error_code":"install_in_progress"}"#,
                        r#"{"code":"install_in_progress"}"#,
                        false,
                    ),
                    HelperScenario::UpdateSuccess => (
                        r#"{"schema_version":1,"sequence":0,"operation":"update","state":"running","phase":"downloading","current_version":"0.1.22","target_version":"0.1.23","bytes_completed":512,"bytes_total":1024}"#,
                        r#"{"schema_version":1,"sequence":1,"operation":"update","state":"succeeded","phase":"completed","current_version":"0.1.22","target_version":"0.1.23"}"#,
                        r#"{"code":"installed","from":"0.1.22","to":"0.1.23"}"#,
                        true,
                    ),
                };
                assert_eq!(
                    operation,
                    if matches!(self.scenario, HelperScenario::UpdateSuccess) {
                        CoreOperationKind::Update
                    } else {
                        CoreOperationKind::Install
                    }
                );
                let running = progress(running);
                let terminal = progress(terminal);
                progress_sender
                    .send(running)
                    .await
                    .map_err(|_| RunnerError::Wait)?;
                tokio::time::sleep(Duration::from_millis(500)).await;
                progress_sender
                    .send(terminal.clone())
                    .await
                    .map_err(|_| RunnerError::Wait)?;
                drop(progress_sender);
                ChildCompletion::with_progress(exit_success, stdout.as_bytes(), [terminal])
            })
        }

        fn check_update(self: Arc<Self>, _executable: PathBuf) -> super::CheckFuture {
            Box::pin(async { Err(RunnerError::Spawn) })
        }
    }

    fn successful_install() -> Result<ChildCompletion, RunnerError> {
        ChildCompletion::with_progress(
            true,
            br#"{"code":"running"}"#,
            [progress(
                r#"{"schema_version":1,"sequence":0,"operation":"install","state":"succeeded","phase":"completed"}"#,
            )],
        )
    }

    fn successful_update() -> Result<ChildCompletion, RunnerError> {
        ChildCompletion::with_progress(
            true,
            br#"{"code":"installed","from":"0.1.22","to":"0.1.23"}"#,
            [progress(
                r#"{"schema_version":1,"sequence":0,"operation":"update","state":"succeeded","phase":"completed","current_version":"0.1.22","target_version":"0.1.23"}"#,
            )],
        )
    }

    fn progress(json: &str) -> ChildProgress {
        serde_json::from_str(json).unwrap()
    }

    async fn wait_for_terminal(coordinator: &CoreOperationCoordinator) -> CoreOperationSnapshot {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(snapshot) = coordinator.status().await
                    && snapshot.state != CoreOperationState::Running
                {
                    return snapshot;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap()
    }

    struct FixedSelector {
        calls: Arc<AtomicUsize>,
        runtime: Mutex<Option<SelectedWokCoreRuntime>>,
    }

    impl DesktopRuntimeSelector for FixedSelector {
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
            let runtime = self.runtime.lock().unwrap().take().unwrap();
            Box::pin(async move { Ok(runtime) })
        }
    }

    fn cached_runtime(
        runtime: SelectedWokCoreRuntime,
    ) -> (Arc<DesktopRuntimeState>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let selector = Arc::new(FixedSelector {
            calls: calls.clone(),
            runtime: Mutex::new(Some(runtime)),
        });
        (
            Arc::new(DesktopRuntimeState::new_with_selector(selector)),
            calls,
        )
    }

    struct PanicAuthority;

    impl TrustedRuntimeAuthority for PanicAuthority {
        fn discover(&self) -> Result<Option<PathBuf>, CoreOperationError> {
            panic!("trusted install-record discovery must not run")
        }
    }

    struct CountingAuthority {
        calls: AtomicUsize,
    }

    impl CountingAuthority {
        fn panic() -> Self {
            Self {
                calls: AtomicUsize::new(0),
            }
        }
    }

    impl TrustedRuntimeAuthority for CountingAuthority {
        fn discover(&self) -> Result<Option<PathBuf>, CoreOperationError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            panic!("trusted install-record discovery must not run")
        }
    }

    struct FakeRunner {
        plans: Mutex<VecDeque<RunPlan>>,
        checks: Mutex<VecDeque<Result<CheckCompletion, RunnerError>>>,
        requests: Mutex<Vec<OperationRequest>>,
        check_executables: Mutex<Vec<PathBuf>>,
        operation_executables: Mutex<Vec<PathBuf>>,
        spawn_count: AtomicUsize,
        reaped_count: AtomicUsize,
        release: Semaphore,
    }

    struct RunPlan {
        blocked: bool,
        completion: Result<ChildCompletion, RunnerError>,
    }

    impl RunPlan {
        fn blocked(completion: Result<ChildCompletion, RunnerError>) -> Self {
            Self {
                blocked: true,
                completion,
            }
        }

        fn immediate(completion: Result<ChildCompletion, RunnerError>) -> Self {
            Self {
                blocked: false,
                completion,
            }
        }
    }

    impl FakeRunner {
        fn new(plans: impl IntoIterator<Item = RunPlan>) -> Self {
            Self {
                plans: Mutex::new(plans.into_iter().collect()),
                checks: Mutex::new(VecDeque::new()),
                requests: Mutex::new(Vec::new()),
                check_executables: Mutex::new(Vec::new()),
                operation_executables: Mutex::new(Vec::new()),
                spawn_count: AtomicUsize::new(0),
                reaped_count: AtomicUsize::new(0),
                release: Semaphore::new(0),
            }
        }

        fn with_checks(
            self,
            checks: impl IntoIterator<Item = Result<CheckCompletion, RunnerError>>,
        ) -> Self {
            *self.checks.lock().unwrap() = checks.into_iter().collect();
            self
        }

        fn spawn_count(&self) -> usize {
            self.spawn_count.load(Ordering::SeqCst)
        }

        fn release_one(&self) {
            self.release.add_permits(1);
        }

        fn executable_requests(&self) -> Vec<PathBuf> {
            let mut paths = self.check_executables.lock().unwrap().clone();
            paths.extend(self.operation_executables.lock().unwrap().clone());
            paths
        }
    }

    impl OperationRunner for FakeRunner {
        fn run(
            self: Arc<Self>,
            request: OperationRequest,
            progress: mpsc::Sender<ChildProgress>,
        ) -> OperationFuture {
            self.spawn_count.fetch_add(1, Ordering::SeqCst);
            if let OperationRequest::Update { executable } = &request {
                self.operation_executables
                    .lock()
                    .unwrap()
                    .push(executable.clone());
            }
            self.requests.lock().unwrap().push(request);
            let plan = self.plans.lock().unwrap().pop_front().unwrap();
            Box::pin(async move {
                if plan.blocked {
                    self.release.acquire().await.unwrap().forget();
                }
                if let Ok(completion) = &plan.completion {
                    for event in completion.progress() {
                        let _ = progress.send(event.clone()).await;
                    }
                    self.reaped_count.fetch_add(1, Ordering::SeqCst);
                }
                plan.completion
            })
        }

        fn check_update(self: Arc<Self>, executable: PathBuf) -> super::CheckFuture {
            self.spawn_count.fetch_add(1, Ordering::SeqCst);
            self.check_executables.lock().unwrap().push(executable);
            let result = self.checks.lock().unwrap().pop_front().unwrap();
            Box::pin(async move { result })
        }
    }

    #[derive(Default)]
    struct RecordingSink {
        snapshots: Mutex<Vec<CoreOperationSnapshot>>,
    }

    impl OperationEventSink for RecordingSink {
        fn emit<'a>(&'a self, snapshot: &'a CoreOperationSnapshot) -> super::EventFuture<'a> {
            Box::pin(async move {
                self.snapshots.lock().unwrap().push(snapshot.clone());
            })
        }
    }

    #[derive(Default)]
    struct BlockingSink {
        terminal_entered: Notify,
        release: Notify,
        terminal_completed: Notify,
        finished: AtomicBool,
    }

    impl OperationEventSink for BlockingSink {
        fn emit<'a>(&'a self, snapshot: &'a CoreOperationSnapshot) -> super::EventFuture<'a> {
            Box::pin(async move {
                if snapshot.state == CoreOperationState::Running {
                    return;
                }
                self.terminal_entered.notify_one();
                self.release.notified().await;
                self.finished.store(true, Ordering::SeqCst);
                self.terminal_completed.notify_one();
            })
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
            let path = self.root.path().join(platform_executable_path(relative));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"synthetic executable").unwrap();
            secure_private_file(&path).unwrap();
            path
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

        async fn development_runtime(&self) -> SelectedWokCoreRuntime {
            let executable = self.create_file("development/wokcore");
            fs::write(
                &self.paths.wokcore_discovery_file,
                serde_json::to_vec(&serde_json::json!({
                    "base_url": "http://127.0.0.1:9",
                    "pid": 41,
                    "instance_id": "01234567-89ab-4cde-8fab-0123456789ab",
                    "wokcore_version": "0.1.0",
                    "api_major": 1
                }))
                .unwrap(),
            )
            .unwrap();
            secure_private_file(&self.paths.wokcore_discovery_file).unwrap();
            RuntimeSelectorHarness::new_with_connection_probe(
                Some(executable.into_os_string()),
                |_process_id, _candidate| true,
                |_record| panic!("production discovery must not run"),
                |_client: WokCoreClient| async { CoreConnection::Stopped },
            )
            .select(&self.paths)
            .await
            .unwrap()
        }

        async fn production_runtime(&self, executable: Option<PathBuf>) -> SelectedWokCoreRuntime {
            RuntimeSelectorHarness::new(
                None,
                |_process_id, _candidate| false,
                move |_record| Ok(executable.clone()),
            )
            .select(&self.paths)
            .await
            .unwrap()
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
