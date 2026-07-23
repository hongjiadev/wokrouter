use wokrouter_control::{ControlError, ControlRequest, ControlResponse, DaemonState, DaemonStatus};
use wokrouter_platform::AppPaths;

use super::{CommandError, endpoint, request};

const NOT_RUNNING_EXIT_CODE: u8 = 3;

pub async fn execute(paths: &AppPaths, json: bool) -> Result<u8, CommandError> {
    let endpoint = endpoint(paths)?;
    let status = match request(&endpoint, ControlRequest::Status).await {
        Ok(ControlResponse::Status(status)) => status,
        Ok(response) => return Err(CommandError::UnexpectedResponse { response }),
        Err(CommandError::Control(ControlError::EndpointUnavailable)) => DaemonStatus {
            state: DaemonState::Stopped,
            version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        Err(error) => return Err(error),
    };

    if json {
        println!(
            "{}",
            serde_json::to_string(&status).expect("daemon status is serializable")
        );
    } else {
        match status.state {
            DaemonState::Running => {
                println!("WokRouter daemon is running (version {}).", status.version)
            }
            DaemonState::Stopped => {
                println!("WokRouter daemon is stopped (version {}).", status.version)
            }
        }
    }

    Ok(if status.state == DaemonState::Stopped {
        NOT_RUNNING_EXIT_CODE
    } else {
        0
    })
}
