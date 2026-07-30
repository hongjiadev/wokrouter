use std::{
    ffi::OsStr,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use tokio::{
    process::{Child, Command},
    sync::{Mutex, watch},
};
use wokrouter_cli::commands::{CoreStatus, CoreUiState, status::snapshot};
use wokrouter_platform::AppPaths;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(6);

type ControlFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

trait StatusReader: Send + Sync {
    fn status(&self) -> ControlFuture<'_, Result<CoreStatus, DesktopControlError>>;
}

struct SystemStatusReader {
    paths: AppPaths,
}

impl StatusReader for SystemStatusReader {
    fn status(&self) -> ControlFuture<'_, Result<CoreStatus, DesktopControlError>> {
        Box::pin(async {
            snapshot(&self.paths)
                .await
                .map(|(status, _)| status)
                .map_err(|_| DesktopControlError::StatusUnavailable)
        })
    }
}

trait CliChild: Send {
    fn wait(&mut self) -> ControlFuture<'_, Result<bool, DesktopControlError>>;
    fn kill_and_wait(&mut self) -> ControlFuture<'_, ()>;
}

trait CliRunner: Send + Sync {
    fn spawn(
        &self,
        executable: &Path,
        action: CliAction,
    ) -> Result<Box<dyn CliChild>, DesktopControlError>;
}

struct SystemCliRunner;

impl CliRunner for SystemCliRunner {
    fn spawn(
        &self,
        executable: &Path,
        action: CliAction,
    ) -> Result<Box<dyn CliChild>, DesktopControlError> {
        let child = cli_command(executable.as_os_str(), action)
            .spawn()
            .map_err(|_| action.error())?;
        Ok(Box::new(SystemCliChild { child }))
    }
}

struct SystemCliChild {
    child: Child,
}

impl CliChild for SystemCliChild {
    fn wait(&mut self) -> ControlFuture<'_, Result<bool, DesktopControlError>> {
        Box::pin(async {
            self.child
                .wait()
                .await
                .map(|status| status.success())
                .map_err(|_| DesktopControlError::CommandUnavailable)
        })
    }

    fn kill_and_wait(&mut self) -> ControlFuture<'_, ()> {
        Box::pin(async {
            let _ = self.child.kill().await;
            let _ = self.child.wait().await;
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CliAction {
    Start,
    Stop,
}

impl CliAction {
    const fn argument(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
        }
    }

    const fn error(self) -> DesktopControlError {
        match self {
            Self::Start => DesktopControlError::StartUnavailable,
            Self::Stop => DesktopControlError::StopUnavailable,
        }
    }
}

type StartResult = Result<(), DesktopControlError>;

#[derive(Default)]
struct StartGate {
    in_flight: Option<watch::Receiver<Option<StartResult>>>,
}

#[derive(Clone)]
pub(crate) struct DesktopControl {
    cli_executable: PathBuf,
    status_reader: Arc<dyn StatusReader>,
    runner: Arc<dyn CliRunner>,
    start_gate: Arc<Mutex<StartGate>>,
    command_timeout: Duration,
}

impl DesktopControl {
    pub(crate) fn discover() -> Result<Self, DesktopControlError> {
        let paths = AppPaths::discover().map_err(|_| DesktopControlError::Initialization)?;
        let current_executable =
            std::env::current_exe().map_err(|_| DesktopControlError::Initialization)?;
        let cli_executable =
            current_executable.with_file_name(format!("wokrouter{}", std::env::consts::EXE_SUFFIX));
        Ok(Self::new_with_dependencies(
            cli_executable,
            Arc::new(SystemStatusReader { paths }),
            Arc::new(SystemCliRunner),
            COMMAND_TIMEOUT,
        ))
    }

    fn new_with_dependencies(
        cli_executable: PathBuf,
        status_reader: Arc<dyn StatusReader>,
        runner: Arc<dyn CliRunner>,
        command_timeout: Duration,
    ) -> Self {
        Self {
            cli_executable,
            status_reader,
            runner,
            start_gate: Arc::new(Mutex::new(StartGate::default())),
            command_timeout,
        }
    }

    pub(crate) async fn status(&self) -> Result<CoreStatus, DesktopControlError> {
        self.status_reader.status().await
    }

    pub(crate) async fn start(&self) -> StartResult {
        let mut receiver = {
            let mut gate = self.start_gate.lock().await;
            if let Some(receiver) = &gate.in_flight {
                receiver.clone()
            } else {
                let (sender, receiver) = watch::channel(None);
                gate.in_flight = Some(receiver.clone());
                let control = self.clone();
                tokio::spawn(async move {
                    let result = control.start_once().await;
                    control.start_gate.lock().await.in_flight = None;
                    sender.send_replace(Some(result));
                });
                receiver
            }
        };

        loop {
            if let Some(result) = { *receiver.borrow() } {
                return result;
            }
            receiver
                .changed()
                .await
                .map_err(|_| DesktopControlError::StartUnavailable)?;
        }
    }

    async fn start_once(&self) -> StartResult {
        if self.status().await?.state == CoreUiState::Running {
            return Ok(());
        }
        self.run_cli(CliAction::Start).await?;
        match self.status().await {
            Ok(status) if status.state == CoreUiState::Running => Ok(()),
            _ => Err(DesktopControlError::StartUnavailable),
        }
    }

    pub(crate) async fn stop(&self) -> Result<(), DesktopControlError> {
        if matches!(
            self.status().await?.state,
            CoreUiState::Missing | CoreUiState::Stopped
        ) {
            return Ok(());
        }
        self.run_cli(CliAction::Stop).await?;
        match self.status().await {
            Ok(status) if matches!(status.state, CoreUiState::Missing | CoreUiState::Stopped) => {
                Ok(())
            }
            _ => Err(DesktopControlError::StopUnavailable),
        }
    }

    async fn run_cli(&self, action: CliAction) -> Result<(), DesktopControlError> {
        let mut child = self.runner.spawn(&self.cli_executable, action)?;
        match tokio::time::timeout(self.command_timeout, child.wait()).await {
            Ok(Ok(true)) => Ok(()),
            Ok(Ok(false)) => Err(action.error()),
            Ok(Err(_)) | Err(_) => {
                child.kill_and_wait().await;
                Err(action.error())
            }
        }
    }
}

fn cli_command(executable: &OsStr, action: CliAction) -> Command {
    let mut command = Command::new(executable);
    command
        .arg(action.argument())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum DesktopControlError {
    #[error("Unable to initialize WokCore control.")]
    Initialization,
    #[error("Unable to read WokCore status. Try again.")]
    StatusUnavailable,
    #[error("WokCore could not start. Try again.")]
    StartUnavailable,
    #[error("WokCore could not stop. Try again.")]
    StopUnavailable,
    #[error("Unable to run the local WokRouter command.")]
    CommandUnavailable,
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeSet, VecDeque},
        ffi::OsStr,
        future::{Future, pending},
        path::{Path, PathBuf},
        pin::Pin,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use super::{
        CliAction, CliChild, CliRunner, DesktopControl, DesktopControlError, StatusReader,
        cli_command,
    };
    use wokrouter_cli::commands::{CoreStatus, CoreUiState};
    use wokrouter_platform::WokCoreRuntimeChannel;

    enum FakeBehavior {
        Fail,
        Hang,
        SucceedAfter(Duration),
    }

    struct FakeStatusReader {
        running: Arc<AtomicBool>,
    }

    impl StatusReader for FakeStatusReader {
        fn status(
            &self,
        ) -> Pin<Box<dyn Future<Output = Result<CoreStatus, DesktopControlError>> + Send + '_>>
        {
            let running = self.running.load(Ordering::SeqCst);
            Box::pin(async move { Ok(status(running)) })
        }
    }

    struct FakeRunner {
        behaviors: Mutex<VecDeque<FakeBehavior>>,
        kill_count: Arc<AtomicUsize>,
        running: Arc<AtomicBool>,
        spawn_count: AtomicUsize,
    }

    impl FakeRunner {
        fn new(
            running: Arc<AtomicBool>,
            behaviors: impl IntoIterator<Item = FakeBehavior>,
        ) -> Self {
            Self {
                behaviors: Mutex::new(behaviors.into_iter().collect()),
                kill_count: Arc::new(AtomicUsize::new(0)),
                running,
                spawn_count: AtomicUsize::new(0),
            }
        }
    }

    impl CliRunner for FakeRunner {
        fn spawn(
            &self,
            _executable: &Path,
            action: CliAction,
        ) -> Result<Box<dyn CliChild>, DesktopControlError> {
            self.spawn_count.fetch_add(1, Ordering::SeqCst);
            let behavior = self
                .behaviors
                .lock()
                .unwrap()
                .pop_front()
                .expect("test runner behavior must be present");
            Ok(Box::new(FakeChild {
                behavior: Some(behavior),
                action,
                kill_count: Arc::clone(&self.kill_count),
                running: Arc::clone(&self.running),
            }))
        }
    }

    struct FakeChild {
        behavior: Option<FakeBehavior>,
        action: CliAction,
        kill_count: Arc<AtomicUsize>,
        running: Arc<AtomicBool>,
    }

    impl CliChild for FakeChild {
        fn wait(
            &mut self,
        ) -> Pin<Box<dyn Future<Output = Result<bool, DesktopControlError>> + Send + '_>> {
            let behavior = self.behavior.take().expect("child wait runs once");
            let action = self.action;
            let running = Arc::clone(&self.running);
            Box::pin(async move {
                match behavior {
                    FakeBehavior::Fail => Ok(false),
                    FakeBehavior::Hang => pending().await,
                    FakeBehavior::SucceedAfter(delay) => {
                        tokio::time::sleep(delay).await;
                        running.store(action == CliAction::Start, Ordering::SeqCst);
                        Ok(true)
                    }
                }
            })
        }

        fn kill_and_wait(&mut self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
            self.kill_count.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {})
        }
    }

    fn control(
        running: Arc<AtomicBool>,
        runner: Arc<FakeRunner>,
        timeout: Duration,
    ) -> DesktopControl {
        DesktopControl::new_with_dependencies(
            PathBuf::from("wokrouter-test"),
            Arc::new(FakeStatusReader { running }),
            runner,
            timeout,
        )
    }

    fn status(running: bool) -> CoreStatus {
        CoreStatus {
            state: if running {
                CoreUiState::Running
            } else {
                CoreUiState::Stopped
            },
            runtime_channel: WokCoreRuntimeChannel::Production,
            version: running.then(|| "0.1.0".to_owned()),
            management_api_major: running.then_some(1),
            capabilities: BTreeSet::new(),
            phase: None,
            active_requests: running.then_some(0),
            error_code: (!running).then_some("not_running"),
        }
    }

    #[test]
    fn cli_boundary_uses_only_fixed_start_and_stop_commands() {
        let executable = Path::new(r"C:\Program Files\WokRouter\wokrouter.exe");

        for (action, argument) in [(CliAction::Start, "start"), (CliAction::Stop, "stop")] {
            let command = cli_command(executable.as_os_str(), action);
            let command = command.as_std();
            assert_eq!(command.get_program(), executable.as_os_str());
            assert_eq!(
                command.get_args().collect::<Vec<_>>(),
                vec![OsStr::new(argument)]
            );
        }
    }

    #[tokio::test]
    async fn concurrent_start_calls_share_one_cli_child() {
        let running = Arc::new(AtomicBool::new(false));
        let runner = Arc::new(FakeRunner::new(
            Arc::clone(&running),
            [FakeBehavior::SucceedAfter(Duration::from_millis(30))],
        ));
        let control = control(Arc::clone(&running), runner.clone(), Duration::from_secs(1));

        let (first, second) = tokio::join!(control.start(), control.start());

        assert_eq!(first, Ok(()));
        assert_eq!(second, Ok(()));
        assert_eq!(runner.spawn_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn hung_start_child_is_killed_and_bounded() {
        let running = Arc::new(AtomicBool::new(false));
        let runner = Arc::new(FakeRunner::new(Arc::clone(&running), [FakeBehavior::Hang]));
        let control = control(
            Arc::clone(&running),
            runner.clone(),
            Duration::from_millis(20),
        );

        assert_eq!(
            control.start().await,
            Err(DesktopControlError::StartUnavailable)
        );
        assert_eq!(runner.kill_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn stop_uses_the_same_bounded_cli_boundary() {
        let running = Arc::new(AtomicBool::new(true));
        let runner = Arc::new(FakeRunner::new(
            Arc::clone(&running),
            [FakeBehavior::SucceedAfter(Duration::ZERO)],
        ));
        let control = control(Arc::clone(&running), runner.clone(), Duration::from_secs(1));

        assert_eq!(control.stop().await, Ok(()));
        assert!(!running.load(Ordering::SeqCst));
        assert_eq!(runner.spawn_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failed_start_can_be_retried() {
        let running = Arc::new(AtomicBool::new(false));
        let runner = Arc::new(FakeRunner::new(
            Arc::clone(&running),
            [
                FakeBehavior::Fail,
                FakeBehavior::SucceedAfter(Duration::ZERO),
            ],
        ));
        let control = control(Arc::clone(&running), runner.clone(), Duration::from_secs(1));

        assert_eq!(
            control.start().await,
            Err(DesktopControlError::StartUnavailable)
        );
        assert_eq!(control.start().await, Ok(()));
        assert_eq!(runner.spawn_count.load(Ordering::SeqCst), 2);
    }
}
