use std::{cmp, io, time::Duration};

use tokio::time::Instant;

use wokrouter_control::{
    ControlClient, ControlEndpoint, ControlError, ControlRequest, ControlResponse,
};
use wokrouter_platform::{AppPaths, PlatformError};

pub mod start;
pub mod status;
pub mod stop;

pub const REQUEST_TIMEOUT: Duration = Duration::from_millis(500);

pub async fn ping_before(
    endpoint: &ControlEndpoint,
    deadline: Instant,
) -> Result<(), CommandError> {
    let response = request_before(endpoint, ControlRequest::Ping, deadline).await?;
    match response {
        ControlResponse::Pong { .. } => Ok(()),
        response => Err(CommandError::UnexpectedResponse { response }),
    }
}

pub async fn request_before(
    endpoint: &ControlEndpoint,
    request: ControlRequest,
    deadline: Instant,
) -> Result<ControlResponse, CommandError> {
    if Instant::now() >= deadline {
        return Err(CommandError::RequestTimedOut);
    }
    tokio::time::timeout_at(deadline, async {
        let client = ControlClient::connect(endpoint).await?;
        client.request(request).await
    })
    .await
    .map_err(|_| CommandError::RequestTimedOut)?
    .map_err(CommandError::from)
}

pub async fn request(
    endpoint: &ControlEndpoint,
    request: ControlRequest,
) -> Result<ControlResponse, CommandError> {
    request_before(endpoint, request, Instant::now() + REQUEST_TIMEOUT).await
}

pub async fn connect_before(
    endpoint: &ControlEndpoint,
    deadline: Instant,
) -> Result<ControlClient, CommandError> {
    if Instant::now() >= deadline {
        return Err(CommandError::RequestTimedOut);
    }
    tokio::time::timeout_at(deadline, ControlClient::connect(endpoint))
        .await
        .map_err(|_| CommandError::RequestTimedOut)?
        .map_err(CommandError::from)
}

pub fn operation_deadline(overall_deadline: Instant) -> Instant {
    cmp::min(overall_deadline, Instant::now() + REQUEST_TIMEOUT)
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
