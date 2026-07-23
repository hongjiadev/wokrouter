use wokrouter_control::{
    ControlClient, ControlError, ControlRequest, ControlResponse, DaemonState, DaemonStatus,
};
use wokrouter_platform::AppPaths;

use super::{CommandError, endpoint};

const NOT_RUNNING_EXIT_CODE: u8 = 3;

pub async fn execute(paths: &AppPaths, json: bool) -> Result<u8, CommandError> {
    let endpoint = endpoint(paths)?;
    let status = match ControlClient::connect(&endpoint).await {
        Ok(client) => match client.request(ControlRequest::Status).await? {
            ControlResponse::Status(status) => status,
            response => return Err(CommandError::UnexpectedResponse { response }),
        },
        Err(ControlError::EndpointUnavailable) => DaemonStatus {
            state: DaemonState::Stopped,
            version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        Err(error) => return Err(error.into()),
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
