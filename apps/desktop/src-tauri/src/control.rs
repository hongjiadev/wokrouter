use std::{ffi::OsStr, path::PathBuf, process::Command};

use serde::Serialize;
use wokrouter_control::{
    ControlClient, ControlEndpoint, ControlError, ControlRequest, ControlResponse, DaemonState,
    DaemonStatus,
};
use wokrouter_platform::AppPaths;

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

pub(crate) struct DesktopControl {
    endpoint: ControlEndpoint,
    cli_executable: PathBuf,
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
        Self {
            endpoint,
            cli_executable: cli_executable.into(),
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

    pub(crate) async fn start(&self) -> Result<(), DesktopControlError> {
        let executable = self.cli_executable.clone();
        let status = tokio::task::spawn_blocking(move || {
            let mut command = start_command(executable.as_os_str());
            command.status()
        })
        .await
        .map_err(|_| DesktopControlError::StartUnavailable)?
        .map_err(|_| DesktopControlError::StartUnavailable)?;
        if status.success() {
            Ok(())
        } else {
            Err(DesktopControlError::StartUnavailable)
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
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

#[derive(Debug, thiserror::Error)]
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
    use std::{ffi::OsStr, path::Path};

    use wokrouter_control::{
        ControlEndpoint, ControlError, ControlResponse, ControlServer, DaemonState, DaemonStatus,
    };

    use super::{DaemonStatusDto, DesktopControl, start_command};

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

        assert_eq!(command.get_program(), executable.as_os_str());
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![OsStr::new("start")]
        );
    }
}
