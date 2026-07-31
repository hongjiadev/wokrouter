mod parser;

use std::{env, future::Future, path::PathBuf, pin::Pin, process::Stdio, sync::Arc};

use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    sync::{Mutex, mpsc},
};
use uuid::Uuid;
use wokrouter_platform::{
    AppPaths, PlatformError, WokCoreRuntimeChannel, discover_wokcore_executable,
};

use self::parser::{ChildProgress, MAX_BUFFER_BYTES, ProgressParser};
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
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
}

impl CoreOperationCoordinator {
    pub(crate) fn new(runtime: Arc<DesktopRuntimeState>) -> Self {
        Self::new_with_dependencies(
            runtime,
            Arc::new(SystemOperationRunner),
            Arc::new(SystemTrustedRuntimeAuthority::discover()),
        )
    }

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
        }
    }

    pub(crate) async fn status(&self) -> Option<CoreOperationSnapshot> {
        self.state.lock().await.last_snapshot.clone()
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
        if self.state.lock().await.active.is_some() {
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
        if self.state.lock().await.active.is_some() {
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
        let runner_failure = result.as_ref().err().copied();
        let final_child = match &result {
            Ok(completion)
                if completion.progress_valid
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
                if terminal_is_accepted {
                    Some(event)
                } else {
                    None
                }
            }
            _ => None,
        };
        let (snapshot, emit) = {
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
            let snapshot = match final_child {
                Some(child) => CoreOperationSnapshot::from_child(operation_id, sequence, child),
                None => {
                    let error_code = match runner_failure {
                        Some(RunnerError::Spawn | RunnerError::Wait | RunnerError::Read) => {
                            operation_failure_code(operation)
                        }
                        _ => "invalid_progress",
                    };
                    CoreOperationSnapshot::failed(operation_id, sequence, operation, error_code)
                }
            };
            state.active = None;
            state.last_snapshot = Some(snapshot.clone());
            (snapshot, true)
        };
        if emit {
            sink.emit(&snapshot).await;
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
        ffi::OsStr,
        fs,
        future::Future,
        path::{Path, PathBuf},
        pin::Pin,
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
        test_support::{RuntimeSelectorHarness, secure_private_file},
    };
    use wokrouter_wokcore_client::{CoreConnection, WokCoreClient};

    use super::{
        CheckCompletion, ChildCompletion, CoreOperationCoordinator, CoreOperationError,
        CoreOperationKind, CoreOperationPhase, CoreOperationSnapshot, CoreOperationState,
        OperationEventSink, OperationFuture, OperationRequest, OperationRunner, RunnerError,
        TrustedRuntimeAuthority, parser::ChildProgress,
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
