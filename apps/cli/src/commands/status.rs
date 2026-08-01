use wokrouter_platform::{
    AppPaths, SelectedWokCoreRuntime, WokCoreRuntimeChannel, select_wokcore_runtime,
};
use wokrouter_wokcore_client::{CoreConnection, ServiceError};

use super::{
    AUTHORIZATION_REQUIRED_EXIT_CODE, CommandError, CommandRuntime, CoreStatus, CoreUiState,
    NOT_RUNNING_EXIT_CODE, load_token, protected_status, public_status,
};

pub async fn execute(runtime: &SelectedWokCoreRuntime, json: bool) -> Result<u8, CommandError> {
    let (status, exit_code) = snapshot_selected(runtime).await?;
    render(&status, json);
    Ok(exit_code)
}

pub async fn snapshot(paths: &AppPaths) -> Result<(CoreStatus, u8), CommandError> {
    let runtime = select_wokcore_runtime(paths).await?;
    snapshot_selected(&runtime).await
}

pub async fn snapshot_selected(
    runtime: &SelectedWokCoreRuntime,
) -> Result<(CoreStatus, u8), CommandError> {
    snapshot_runtime(runtime).await
}

async fn snapshot_runtime(runtime: &impl CommandRuntime) -> Result<(CoreStatus, u8), CommandError> {
    let runtime_channel = runtime.channel();
    let client = runtime.client();
    if runtime_channel == WokCoreRuntimeChannel::Production && runtime.executable().is_none() {
        let status = CoreStatus::missing(runtime_channel);
        return Ok((status, NOT_RUNNING_EXIT_CODE));
    }
    let connection = runtime.connection().await;
    let (status, exit_code) = match connection {
        CoreConnection::Running(handshake) => match load_token().await? {
            None => (
                public_status(runtime_channel, CoreConnection::Running(handshake)),
                AUTHORIZATION_REQUIRED_EXIT_CODE,
            ),
            Some(token) => match client.service_status(&token).await {
                Ok(service) => (protected_status(runtime_channel, handshake, service), 0),
                Err(ServiceError::Unauthorized | ServiceError::Forbidden) => (
                    public_status(runtime_channel, CoreConnection::Running(handshake)),
                    AUTHORIZATION_REQUIRED_EXIT_CODE,
                ),
                Err(error) => return Err(error.into()),
            },
        },
        other => {
            let status = public_status(runtime_channel, other);
            let exit = match status.state {
                CoreUiState::Stopped | CoreUiState::Missing => NOT_RUNNING_EXIT_CODE,
                CoreUiState::Incompatible | CoreUiState::InvalidRuntime => 1,
                _ => 0,
            };
            (status, exit)
        }
    };
    Ok((status, exit_code))
}

fn render(status: &CoreStatus, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string(status).expect("core status is serializable")
        );
        return;
    }
    match status.state {
        CoreUiState::Missing => println!("WokCore is not installed."),
        CoreUiState::Stopped => println!("WokCore is stopped."),
        CoreUiState::Starting => println!("WokCore is starting."),
        CoreUiState::Running => println!(
            "WokCore is running (version {}).",
            status.version.as_deref().unwrap_or("unknown")
        ),
        CoreUiState::Draining => println!("WokCore is draining active requests."),
        CoreUiState::AuthorizationRequired => {
            println!("WokCore is running, but WokRouter authorization is required.")
        }
        CoreUiState::Incompatible => println!("WokCore uses an incompatible API version."),
        CoreUiState::InvalidRuntime => println!("WokCore runtime metadata is invalid."),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        sync::{
            OnceLock,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use wokrouter_platform::WokCoreRuntimeChannel;
    use wokrouter_wokcore_client::{Compatibility, CoreConnection, WokCoreClient};

    use super::snapshot_runtime;
    use crate::commands::{CommandRuntime, CoreUiState};

    struct FakeRuntime {
        channel: WokCoreRuntimeChannel,
        client: WokCoreClient,
        connection: CoreConnection,
    }

    impl FakeRuntime {
        fn new(channel: WokCoreRuntimeChannel, connection: CoreConnection) -> Self {
            Self {
                channel,
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
            None
        }

        fn client(&self) -> &WokCoreClient {
            &self.client
        }

        fn establish_production_binding(&self, _executable: &Path) -> bool {
            false
        }

        async fn connection(&self) -> CoreConnection {
            self.connection.clone()
        }
    }

    struct RefreshingProductionRuntime {
        client: WokCoreClient,
        executable: OnceLock<PathBuf>,
        refresh_calls: AtomicUsize,
    }

    impl RefreshingProductionRuntime {
        fn new() -> Self {
            Self {
                client: WokCoreClient::new(PathBuf::from("unused-discovery.json")).unwrap(),
                executable: OnceLock::new(),
                refresh_calls: AtomicUsize::new(0),
            }
        }
    }

    impl CommandRuntime for RefreshingProductionRuntime {
        fn channel(&self) -> WokCoreRuntimeChannel {
            WokCoreRuntimeChannel::Production
        }

        fn executable(&self) -> Option<&Path> {
            self.executable.get().map(PathBuf::as_path)
        }

        fn client(&self) -> &WokCoreClient {
            self.refresh_calls.fetch_add(1, Ordering::SeqCst);
            let _ = self
                .executable
                .set(PathBuf::from("trusted-wokcore-after-install"));
            &self.client
        }

        fn establish_production_binding(&self, _executable: &Path) -> bool {
            false
        }

        async fn connection(&self) -> CoreConnection {
            CoreConnection::Stopped
        }
    }

    #[tokio::test]
    async fn production_status_refreshes_trusted_binding_before_classifying_missing() {
        let runtime = RefreshingProductionRuntime::new();

        let (status, _) = snapshot_runtime(&runtime).await.unwrap();

        assert_eq!(status.state, CoreUiState::Stopped);
        assert_eq!(runtime.refresh_calls.load(Ordering::SeqCst), 1);
        assert!(runtime.executable().is_some());
    }

    #[tokio::test]
    async fn incompatible_and_invalid_development_statuses_keep_the_development_channel() {
        let incompatible = FakeRuntime::new(
            WokCoreRuntimeChannel::Development,
            CoreConnection::Incompatible(Compatibility {
                wokcore_minimum_api_major: 2,
                wokcore_maximum_api_major: 2,
                wokrouter_minimum_api_major: 1,
                wokrouter_maximum_api_major: 1,
            }),
        );
        let invalid = FakeRuntime::new(
            WokCoreRuntimeChannel::Development,
            CoreConnection::InvalidRuntime,
        );

        let (incompatible, incompatible_exit) = snapshot_runtime(&incompatible).await.unwrap();
        let (invalid, invalid_exit) = snapshot_runtime(&invalid).await.unwrap();

        assert_eq!(
            incompatible.runtime_channel,
            WokCoreRuntimeChannel::Development
        );
        assert_eq!(incompatible.state, CoreUiState::Incompatible);
        assert_eq!(incompatible_exit, 1);
        assert_eq!(invalid.runtime_channel, WokCoreRuntimeChannel::Development);
        assert_eq!(invalid.state, CoreUiState::InvalidRuntime);
        assert_eq!(invalid_exit, 1);
    }
}
