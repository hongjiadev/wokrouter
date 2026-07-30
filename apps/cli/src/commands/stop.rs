use std::{path::PathBuf, time::Duration};

use tokio::time::Instant;
use wokrouter_platform::{SelectedWokCoreRuntime, WokCoreRuntimeChannel};
use wokrouter_wokcore_client::{CoreConnection, ServiceError, WokCoreClient};

use super::{CommandError, CommandRuntime, authorize, reauthorize};

const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY_DELAY: Duration = Duration::from_millis(50);

pub async fn execute(runtime: &SelectedWokCoreRuntime) -> Result<u8, CommandError> {
    execute_runtime(runtime, &SystemStopActions).await
}

trait StopActions {
    async fn stop(&self, client: &WokCoreClient, executable: PathBuf) -> Result<(), CommandError>;
}

struct SystemStopActions;

impl StopActions for SystemStopActions {
    async fn stop(&self, client: &WokCoreClient, executable: PathBuf) -> Result<(), CommandError> {
        let token = authorize(executable.clone()).await?;
        match client.stop(&token).await {
            Ok(()) => Ok(()),
            Err(ServiceError::Unauthorized | ServiceError::Forbidden) => {
                let token = reauthorize(executable).await?;
                client.stop(&token).await?;
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }
}

async fn execute_runtime(
    runtime: &impl CommandRuntime,
    actions: &impl StopActions,
) -> Result<u8, CommandError> {
    if runtime.channel() == WokCoreRuntimeChannel::Development {
        return Err(CommandError::DevelopmentRuntimeManagedByIde);
    }
    let executable = match runtime.executable() {
        Some(executable) => executable.to_path_buf(),
        None => {
            println!("{}", stop_message(true));
            return Ok(0);
        }
    };
    match runtime.connection().await {
        CoreConnection::Missing | CoreConnection::Stopped => {
            println!("{}", stop_message(true));
            return Ok(0);
        }
        CoreConnection::Incompatible(_) => return Err(CommandError::Incompatible),
        CoreConnection::InvalidRuntime => return Err(CommandError::InvalidRuntime),
        CoreConnection::Running(_) => {}
    }

    actions.stop(runtime.client(), executable).await?;

    let deadline = Instant::now() + STOP_TIMEOUT;
    loop {
        match runtime.connection().await {
            CoreConnection::Missing | CoreConnection::Stopped => {
                println!("{}", stop_message(false));
                return Ok(0);
            }
            CoreConnection::Running(_)
            | CoreConnection::Incompatible(_)
            | CoreConnection::InvalidRuntime => {}
        }
        if Instant::now() >= deadline {
            return Err(CommandError::StopTimedOut);
        }
        tokio::time::sleep_until(std::cmp::min(deadline, Instant::now() + RETRY_DELAY)).await;
    }
}

fn stop_message(already_stopped: bool) -> &'static str {
    if already_stopped {
        "WokCore is already stopped."
    } else {
        "WokCore is stopped."
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        sync::atomic::{AtomicUsize, Ordering},
    };

    use wokrouter_platform::WokCoreRuntimeChannel;
    use wokrouter_wokcore_client::{CoreConnection, WokCoreClient};

    use super::{StopActions, execute_runtime, stop_message};
    use crate::commands::{CommandError, CommandRuntime};

    struct FakeRuntime {
        client: WokCoreClient,
    }

    impl CommandRuntime for FakeRuntime {
        fn channel(&self) -> WokCoreRuntimeChannel {
            WokCoreRuntimeChannel::Development
        }

        fn executable(&self) -> Option<&Path> {
            Some(Path::new(r"C:\work\wokcore.exe"))
        }

        fn client(&self) -> &WokCoreClient {
            &self.client
        }

        async fn connection(&self) -> CoreConnection {
            CoreConnection::Running(wokrouter_wokcore_client::CoreHandshake {
                instance_id: "test-instance".to_owned(),
                installation_id: None,
                version: "0.1.0".to_owned(),
                management_api_major: 1,
                provider_protocols: Default::default(),
                capabilities: Default::default(),
            })
        }
    }

    #[derive(Default)]
    struct RecordingActions {
        stop_count: AtomicUsize,
    }

    impl StopActions for RecordingActions {
        async fn stop(
            &self,
            _client: &WokCoreClient,
            _executable: PathBuf,
        ) -> Result<(), CommandError> {
            self.stop_count.fetch_add(1, Ordering::SeqCst);
            panic!("development runtime must never receive a stop request")
        }
    }

    #[tokio::test]
    async fn every_development_stop_is_left_for_the_ide_without_a_child_action() {
        let runtime = FakeRuntime {
            client: WokCoreClient::new(PathBuf::from("unused-discovery.json")).unwrap(),
        };
        let actions = RecordingActions::default();

        assert_eq!(
            execute_runtime(&runtime, &actions).await,
            Err(CommandError::DevelopmentRuntimeManagedByIde)
        );
        assert_eq!(actions.stop_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn production_stop_human_output_is_unchanged() {
        assert_eq!(stop_message(true), "WokCore is already stopped.");
        assert_eq!(stop_message(false), "WokCore is stopped.");
    }
}
