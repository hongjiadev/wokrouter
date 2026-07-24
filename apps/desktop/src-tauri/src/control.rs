use std::{
    ffi::OsStr,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use serde::Serialize;
use tokio::{
    process::{Child, Command},
    sync::{Mutex, watch},
};
use wokrouter_control::{
    ControlClient, ControlEndpoint, ControlError, ControlRequest, ControlResponse, DaemonState,
    DaemonStatus,
};
use wokrouter_platform::AppPaths;

const START_TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct DaemonStatusDto {
    state: DaemonState,
    version: String,
}

impl From<DaemonStatus> for DaemonStatusDto {
    fn from(status: DaemonStatus) -> Self {
        Self {
            state: status.state,
            version: status.version,
        }
    }
}

type CliFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

trait CliChild: Send {
    fn wait(&mut self) -> CliFuture<'_, Result<bool, DesktopControlError>>;
    fn kill_and_wait(&mut self) -> CliFuture<'_, ()>;
}

trait CliRunner: Send + Sync {
    fn spawn(&self, executable: &Path) -> Result<Box<dyn CliChild>, DesktopControlError>;
}

struct SystemCliRunner;

impl CliRunner for SystemCliRunner {
    fn spawn(&self, executable: &Path) -> Result<Box<dyn CliChild>, DesktopControlError> {
        let mut command = start_command(executable.as_os_str());
        let child = command
            .spawn()
            .map_err(|_| DesktopControlError::StartUnavailable)?;
        Ok(Box::new(SystemCliChild { child }))
    }
}

struct SystemCliChild {
    child: Child,
}

impl CliChild for SystemCliChild {
    fn wait(&mut self) -> CliFuture<'_, Result<bool, DesktopControlError>> {
        Box::pin(async {
            self.child
                .wait()
                .await
                .map(|status| status.success())
                .map_err(|_| DesktopControlError::StartUnavailable)
        })
    }

    fn kill_and_wait(&mut self) -> CliFuture<'_, ()> {
        Box::pin(async {
            let _ = self.child.kill().await;
            let _ = self.child.wait().await;
        })
    }
}

type StartResult = Result<(), DesktopControlError>;

#[derive(Default)]
struct StartGate {
    in_flight: Option<watch::Receiver<Option<StartResult>>>,
}

#[derive(Clone)]
pub(crate) struct DesktopControl {
    endpoint: ControlEndpoint,
    cli_executable: PathBuf,
    runner: Arc<dyn CliRunner>,
    start_gate: Arc<Mutex<StartGate>>,
    start_timeout: Duration,
}

impl DesktopControl {
    pub(crate) fn discover() -> Result<Self, DesktopControlError> {
        let paths = AppPaths::discover().map_err(|_| DesktopControlError::Initialization)?;
        let endpoint = ControlEndpoint::for_runtime_dir(&paths.runtime_dir)
            .map_err(|_| DesktopControlError::Initialization)?;
        let current_executable =
            std::env::current_exe().map_err(|_| DesktopControlError::Initialization)?;
        let cli_executable =
            current_executable.with_file_name(format!("wokrouter{}", std::env::consts::EXE_SUFFIX));
        Ok(Self::new(endpoint, cli_executable))
    }

    pub(crate) fn new(endpoint: ControlEndpoint, cli_executable: impl Into<PathBuf>) -> Self {
        Self::new_with_runner(
            endpoint,
            cli_executable.into(),
            Arc::new(SystemCliRunner),
            START_TIMEOUT,
        )
    }

    fn new_with_runner(
        endpoint: ControlEndpoint,
        cli_executable: PathBuf,
        runner: Arc<dyn CliRunner>,
        start_timeout: Duration,
    ) -> Self {
        Self {
            endpoint,
            cli_executable,
            runner,
            start_gate: Arc::new(Mutex::new(StartGate::default())),
            start_timeout,
        }
    }

    pub(crate) async fn status(&self) -> Result<DaemonStatusDto, DesktopControlError> {
        let client = match ControlClient::connect(&self.endpoint).await {
            Ok(client) => client,
            Err(ControlError::EndpointUnavailable) => return Ok(stopped_status()),
            Err(_) => return Err(DesktopControlError::StatusUnavailable),
        };
        let response = match client.request(ControlRequest::Status).await {
            Ok(response) => response,
            Err(ControlError::EndpointUnavailable) => return Ok(stopped_status()),
            Err(_) => return Err(DesktopControlError::StatusUnavailable),
        };
        match response {
            ControlResponse::Status(status) => Ok(status.into()),
            _ => Err(DesktopControlError::StatusUnavailable),
        }
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
        match self.status().await {
            Ok(status) if status.state == DaemonState::Running => return Ok(()),
            Ok(_) => {}
            Err(_) => return Err(DesktopControlError::StartUnavailable),
        }

        let mut child = self.runner.spawn(&self.cli_executable)?;
        match tokio::time::timeout(self.start_timeout, child.wait()).await {
            Ok(Ok(true)) => {}
            Ok(Ok(false)) => return Err(DesktopControlError::StartUnavailable),
            Ok(Err(_)) | Err(_) => {
                child.kill_and_wait().await;
                return Err(DesktopControlError::StartUnavailable);
            }
        }

        match self.status().await {
            Ok(status) if status.state == DaemonState::Running => Ok(()),
            _ => Err(DesktopControlError::StartUnavailable),
        }
    }
}

fn stopped_status() -> DaemonStatusDto {
    DaemonStatusDto {
        state: DaemonState::Stopped,
        version: env!("CARGO_PKG_VERSION").to_owned(),
    }
}

fn start_command(executable: &OsStr) -> Command {
    let mut command = Command::new(executable);
    command
        .arg("start")
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
    #[error("Unable to initialize desktop control.")]
    Initialization,
    #[error("Unable to read daemon status. Try again.")]
    StatusUnavailable,
    #[error("WokRouter could not start. Try again.")]
    StartUnavailable,
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
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

    use wokrouter_control::{
        ControlEndpoint, ControlError, ControlResponse, ControlServer, DaemonState, DaemonStatus,
    };

    use super::{
        CliChild, CliRunner, DaemonStatusDto, DesktopControl, DesktopControlError, start_command,
    };

    enum FakeBehavior {
        Fail,
        Hang,
        SucceedAfter(Duration),
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
        fn spawn(&self, _executable: &Path) -> Result<Box<dyn CliChild>, DesktopControlError> {
            self.spawn_count.fetch_add(1, Ordering::SeqCst);
            let behavior = self
                .behaviors
                .lock()
                .unwrap()
                .pop_front()
                .expect("test runner behavior must be present");
            Ok(Box::new(FakeChild {
                behavior: Some(behavior),
                kill_count: Arc::clone(&self.kill_count),
                running: Arc::clone(&self.running),
            }))
        }
    }

    struct FakeChild {
        behavior: Option<FakeBehavior>,
        kill_count: Arc<AtomicUsize>,
        running: Arc<AtomicBool>,
    }

    impl CliChild for FakeChild {
        fn wait(
            &mut self,
        ) -> Pin<Box<dyn Future<Output = Result<bool, DesktopControlError>> + Send + '_>> {
            let behavior = self.behavior.take().expect("child wait runs once");
            let running = Arc::clone(&self.running);
            Box::pin(async move {
                match behavior {
                    FakeBehavior::Fail => Ok(false),
                    FakeBehavior::Hang => pending().await,
                    FakeBehavior::SucceedAfter(delay) => {
                        tokio::time::sleep(delay).await;
                        running.store(true, Ordering::SeqCst);
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

    async fn bind_status_server(
        endpoint: ControlEndpoint,
        running: Arc<AtomicBool>,
    ) -> ControlServer {
        ControlServer::bind(endpoint, move |_| {
            let running = running.load(Ordering::SeqCst);
            async move {
                ControlResponse::Status(DaemonStatus {
                    state: if running {
                        DaemonState::Running
                    } else {
                        DaemonState::Stopped
                    },
                    version: "0.1.0".to_owned(),
                })
            }
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn absent_endpoint_maps_to_stopped_desktop_version() {
        let endpoint = ControlEndpoint::temporary("desktop-absent").unwrap();
        let control = DesktopControl::new(endpoint, "wokrouter-test");

        let status = control.status().await.unwrap();

        assert_eq!(
            status,
            DaemonStatusDto {
                state: DaemonState::Stopped,
                version: env!("CARGO_PKG_VERSION").to_owned(),
            }
        );
    }

    #[tokio::test]
    async fn running_status_uses_the_daemon_version() {
        let endpoint = ControlEndpoint::temporary("desktop-running").unwrap();
        let server = ControlServer::bind(endpoint.clone(), |_| async {
            ControlResponse::Status(DaemonStatus {
                state: DaemonState::Running,
                version: "8.4.2".to_owned(),
            })
        })
        .await
        .unwrap();
        let control = DesktopControl::new(endpoint, "wokrouter-test");

        let status = control.status().await.unwrap();

        assert_eq!(
            status,
            DaemonStatusDto {
                state: DaemonState::Running,
                version: "8.4.2".to_owned(),
            }
        );
        server.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn typed_ipc_error_is_mapped_without_wire_details() {
        let endpoint = ControlEndpoint::temporary("desktop-error").unwrap();
        let server = ControlServer::bind(endpoint.clone(), |_| async {
            ControlResponse::Error(ControlError::InvalidFrame {
                message: r"private payload at C:\Users\someone\state.db".to_owned(),
            })
        })
        .await
        .unwrap();
        let control = DesktopControl::new(endpoint, "wokrouter-test");

        let message = control.status().await.unwrap_err().to_string();

        assert_eq!(message, "Unable to read daemon status. Try again.");
        assert!(!message.contains("someone"));
        assert!(!message.contains("state.db"));
        server.shutdown().await.unwrap();
    }

    #[test]
    fn start_boundary_invokes_only_the_sibling_cli_start_command() {
        let executable = Path::new(r"C:\Program Files\WokRouter\wokrouter.exe");

        let command = start_command(executable.as_os_str());
        let command = command.as_std();

        assert_eq!(command.get_program(), executable.as_os_str());
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![OsStr::new("start")]
        );
    }

    #[tokio::test]
    async fn hung_start_child_is_killed_and_bounded() {
        let endpoint = ControlEndpoint::temporary("desktop-start-hung").unwrap();
        let running = Arc::new(AtomicBool::new(false));
        let server = bind_status_server(endpoint.clone(), Arc::clone(&running)).await;
        let runner = Arc::new(FakeRunner::new(Arc::clone(&running), [FakeBehavior::Hang]));
        let control = DesktopControl::new_with_runner(
            endpoint,
            PathBuf::from("wokrouter-test"),
            runner.clone(),
            Duration::from_millis(20),
        );

        let result = tokio::time::timeout(Duration::from_secs(1), control.start())
            .await
            .expect("hung child must be bounded");

        assert_eq!(result, Err(DesktopControlError::StartUnavailable));
        assert_eq!(runner.spawn_count.load(Ordering::SeqCst), 1);
        assert_eq!(runner.kill_count.load(Ordering::SeqCst), 1);
        server.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn concurrent_start_calls_share_one_cli_child() {
        let endpoint = ControlEndpoint::temporary("desktop-start-concurrent").unwrap();
        let running = Arc::new(AtomicBool::new(false));
        let server = bind_status_server(endpoint.clone(), Arc::clone(&running)).await;
        let runner = Arc::new(FakeRunner::new(
            Arc::clone(&running),
            [FakeBehavior::SucceedAfter(Duration::from_millis(40))],
        ));
        let control = DesktopControl::new_with_runner(
            endpoint,
            PathBuf::from("wokrouter-test"),
            runner.clone(),
            Duration::from_secs(1),
        );

        let (first, second) = tokio::join!(control.start(), control.start());

        assert_eq!(first, Ok(()));
        assert_eq!(second, Ok(()));
        assert_eq!(runner.spawn_count.load(Ordering::SeqCst), 1);
        server.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn failed_start_can_be_retried() {
        let endpoint = ControlEndpoint::temporary("desktop-start-retry").unwrap();
        let running = Arc::new(AtomicBool::new(false));
        let server = bind_status_server(endpoint.clone(), Arc::clone(&running)).await;
        let runner = Arc::new(FakeRunner::new(
            Arc::clone(&running),
            [
                FakeBehavior::Fail,
                FakeBehavior::SucceedAfter(Duration::ZERO),
            ],
        ));
        let control = DesktopControl::new_with_runner(
            endpoint,
            PathBuf::from("wokrouter-test"),
            runner.clone(),
            Duration::from_secs(1),
        );

        assert_eq!(
            control.start().await,
            Err(DesktopControlError::StartUnavailable)
        );
        assert_eq!(control.start().await, Ok(()));
        assert_eq!(runner.spawn_count.load(Ordering::SeqCst), 2);
        server.shutdown().await.unwrap();
    }
}
