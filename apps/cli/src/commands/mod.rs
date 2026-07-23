use std::{io, time::Duration};

use wokrouter_control::{
    ControlClient, ControlEndpoint, ControlError, ControlRequest, ControlResponse,
};
use wokrouter_platform::{AppPaths, PlatformError};

pub mod start;
pub mod status;
pub mod stop;

const REQUEST_TIMEOUT: Duration = Duration::from_millis(500);

pub async fn ping(endpoint: &ControlEndpoint) -> Result<(), CommandError> {
    let response = tokio::time::timeout(REQUEST_TIMEOUT, async {
        let client = ControlClient::connect(endpoint).await?;
        client.request(ControlRequest::Ping).await
    })
    .await
    .map_err(|_| CommandError::RequestTimedOut)??;
    match response {
        ControlResponse::Pong { .. } => Ok(()),
        response => Err(CommandError::UnexpectedResponse { response }),
    }
}

pub fn endpoint(paths: &AppPaths) -> Result<ControlEndpoint, CommandError> {
    Ok(ControlEndpoint::for_runtime_dir(&paths.runtime_dir)?)
}

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("usage: wokrouter <start|status [--json]|stop>")]
    Usage,
    #[error(transparent)]
    Platform(#[from] PlatformError),
    #[error(transparent)]
    Control(#[from] ControlError),
    #[error("CLI I/O failed: {source}")]
    Io {
        #[from]
        source: io::Error,
    },
    #[error("control request timed out")]
    RequestTimedOut,
    #[error("daemon did not become ready within five seconds")]
    StartTimedOut,
    #[error("daemon did not close its control endpoint within five seconds")]
    StopTimedOut,
    #[error("daemon failed to start: {message}")]
    DaemonFailed { message: String },
    #[error("daemon returned an unexpected control response: {response:?}")]
    UnexpectedResponse { response: ControlResponse },
}
