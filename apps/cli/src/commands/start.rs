use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

use tokio::time::Instant;
use wokrouter_platform::{SelectedWokCoreRuntime, WokCoreRuntimeChannel};
use wokrouter_wokcore_client::{CoreConnection, ServiceError, WokCoreClient};

use super::{CommandError, CommandRuntime, authorize, reauthorize};

const START_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY_DELAY: Duration = Duration::from_millis(50);

pub async fn execute(runtime: &SelectedWokCoreRuntime) -> Result<u8, CommandError> {
    execute_runtime(runtime, &SystemStartActions).await
}

trait StartActions {
    async fn ensure_authorized(
        &self,
        client: &WokCoreClient,
        executable: PathBuf,
    ) -> Result<(), CommandError>;

    fn spawn_core(&self, executable: &Path) -> Result<Child, CommandError>;
}

struct SystemStartActions;

impl StartActions for SystemStartActions {
    async fn ensure_authorized(
        &self,
        client: &WokCoreClient,
        executable: PathBuf,
    ) -> Result<(), CommandError> {
        ensure_authorized(client, executable).await
    }

    fn spawn_core(&self, executable: &Path) -> Result<Child, CommandError> {
        spawn_core(executable)
    }
}

async fn execute_runtime(
    runtime: &impl CommandRuntime,
    actions: &impl StartActions,
) -> Result<u8, CommandError> {
    let executable = runtime
        .executable()
        .map(Path::to_path_buf)
        .ok_or(CommandError::WokCoreMissing)?;
    let connection = runtime.connection().await;
    if runtime.channel() == WokCoreRuntimeChannel::Development {
        return match connection {
            CoreConnection::Running(_) => {
                actions
                    .ensure_authorized(runtime.client(), executable)
                    .await?;
                println!("{}", start_message(true));
                Ok(0)
            }
            CoreConnection::Incompatible(_) => Err(CommandError::Incompatible),
            CoreConnection::InvalidRuntime => Err(CommandError::InvalidRuntime),
            CoreConnection::Missing | CoreConnection::Stopped => {
                Err(CommandError::DevelopmentRuntimeManagedByIde)
            }
        };
    }

    if let CoreConnection::Running(_) = connection {
        actions
            .ensure_authorized(runtime.client(), executable)
            .await?;
        println!("{}", start_message(true));
        return Ok(0);
    }

    let mut child = actions.spawn_core(&executable)?;
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        match runtime.connection().await {
            CoreConnection::Running(_) => {
                actions
                    .ensure_authorized(runtime.client(), executable)
                    .await?;
                println!("{}", start_message(false));
                return Ok(0);
            }
            CoreConnection::Incompatible(_) => {
                kill_created_child(&mut child);
                return Err(CommandError::Incompatible);
            }
            CoreConnection::InvalidRuntime => {
                kill_created_child(&mut child);
                return Err(CommandError::InvalidRuntime);
            }
            CoreConnection::Missing | CoreConnection::Stopped => {}
        }

        if Instant::now() >= deadline {
            let child_exited = child.try_wait().is_ok_and(|status| status.is_some());
            kill_created_child(&mut child);
            return Err(if child_exited {
                CommandError::StartFailed
            } else {
                CommandError::StartTimedOut
            });
        }
        tokio::time::sleep_until(std::cmp::min(deadline, Instant::now() + RETRY_DELAY)).await;
    }
}

async fn ensure_authorized(
    client: &wokrouter_wokcore_client::WokCoreClient,
    executable: std::path::PathBuf,
) -> Result<(), CommandError> {
    let token = authorize(executable.clone()).await?;
    match client.service_status(&token).await {
        Ok(_) => Ok(()),
        Err(ServiceError::Unauthorized | ServiceError::Forbidden) => {
            let token = reauthorize(executable).await?;
            client.service_status(&token).await?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn spawn_core(executable: &Path) -> Result<Child, CommandError> {
    spawn_command(executable)
        .spawn()
        .map_err(|_| CommandError::StartFailed)
}

fn spawn_command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    command
        .args(["serve", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn kill_created_child(child: &mut Child) {
    if child.try_wait().is_ok_and(|status| status.is_none()) {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn start_message(already_running: bool) -> &'static str {
    if already_running {
        "WokCore is already running."
    } else {
        "WokCore is running."
    }
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsStr,
        path::{Path, PathBuf},
        sync::{
            Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };

    use wokrouter_platform::WokCoreRuntimeChannel;
    use wokrouter_wokcore_client::{CoreConnection, WokCoreClient};

    use super::{StartActions, execute_runtime, spawn_command, start_message};
    use crate::commands::{CommandError, CommandRuntime};

    struct FakeRuntime {
        channel: WokCoreRuntimeChannel,
        executable: PathBuf,
        client: WokCoreClient,
        connection: CoreConnection,
    }

    impl FakeRuntime {
        fn development(connection: CoreConnection) -> Self {
            Self {
                channel: WokCoreRuntimeChannel::Development,
                executable: PathBuf::from(r"C:\work\wokcore.exe"),
                client: WokCoreClient::new(PathBuf::from("unused-discovery.json")).unwrap(),
                connection,
            }
        }
    }

    impl CommandRuntime for FakeRuntime {
        fn channel(&self) -> WokCoreRuntimeChannel {
            self.channel
        }

        fn executable(&self) -> Option<&Path> {
            Some(&self.executable)
        }

        fn client(&self) -> &WokCoreClient {
            &self.client
        }

        async fn connection(&self) -> CoreConnection {
            self.connection.clone()
        }
    }

    #[derive(Default)]
    struct RecordingActions {
        authorized_executable: Mutex<Option<PathBuf>>,
        authorized_selected_client: AtomicBool,
        expected_client: Mutex<Option<usize>>,
        spawn_count: AtomicUsize,
    }

    impl RecordingActions {
        fn expect_client(&self, client: &WokCoreClient) {
            *self.expected_client.lock().unwrap() = Some(client as *const WokCoreClient as usize);
        }
    }

    impl StartActions for RecordingActions {
        async fn ensure_authorized(
            &self,
            client: &WokCoreClient,
            executable: PathBuf,
        ) -> Result<(), CommandError> {
            *self.authorized_executable.lock().unwrap() = Some(executable);
            let expected = self.expected_client.lock().unwrap().unwrap();
            self.authorized_selected_client.store(
                client as *const WokCoreClient as usize == expected,
                Ordering::SeqCst,
            );
            Ok(())
        }

        fn spawn_core(&self, _executable: &Path) -> Result<std::process::Child, CommandError> {
            self.spawn_count.fetch_add(1, Ordering::SeqCst);
            panic!("development runtime must never spawn WokCore")
        }
    }

    #[tokio::test]
    async fn running_development_runtime_authorizes_the_selected_client_without_spawning() {
        let runtime = FakeRuntime::development(CoreConnection::Running(handshake()));
        let actions = RecordingActions::default();
        actions.expect_client(runtime.client());

        assert_eq!(execute_runtime(&runtime, &actions).await, Ok(0));
        assert_eq!(
            *actions.authorized_executable.lock().unwrap(),
            Some(runtime.executable.clone())
        );
        assert!(actions.authorized_selected_client.load(Ordering::SeqCst));
        assert_eq!(actions.spawn_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn stopped_development_runtime_is_left_for_the_ide_without_spawning() {
        for connection in [CoreConnection::Missing, CoreConnection::Stopped] {
            let runtime = FakeRuntime::development(connection);
            let actions = RecordingActions::default();

            assert_eq!(
                execute_runtime(&runtime, &actions).await,
                Err(CommandError::DevelopmentRuntimeManagedByIde)
            );
            assert_eq!(actions.spawn_count.load(Ordering::SeqCst), 0);
            assert!(actions.authorized_executable.lock().unwrap().is_none());
        }
    }

    #[test]
    fn start_process_contains_only_the_fixed_serve_command() {
        let executable = Path::new(r"C:\Program Files\WokCore\wokcore.exe");
        let command = spawn_command(executable);

        assert_eq!(command.get_program(), executable.as_os_str());
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![OsStr::new("serve"), OsStr::new("--json")]
        );
    }

    #[test]
    fn production_start_human_output_is_unchanged() {
        assert_eq!(start_message(true), "WokCore is already running.");
        assert_eq!(start_message(false), "WokCore is running.");
    }

    fn handshake() -> wokrouter_wokcore_client::CoreHandshake {
        wokrouter_wokcore_client::CoreHandshake {
            instance_id: "test-instance".to_owned(),
            installation_id: None,
            version: "0.1.0".to_owned(),
            management_api_major: 1,
            provider_protocols: Default::default(),
            capabilities: Default::default(),
        }
    }
}
