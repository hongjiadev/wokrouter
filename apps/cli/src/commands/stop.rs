use std::time::Duration;

use tokio::time::Instant;
use wokrouter_control::{ControlError, ControlRequest, ControlResponse};
use wokrouter_platform::AppPaths;

use super::{CommandError, connect_before, endpoint, operation_deadline, request};

const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY_DELAY: Duration = Duration::from_millis(50);

pub async fn execute(paths: &AppPaths) -> Result<u8, CommandError> {
    let endpoint = endpoint(paths)?;
    match request(&endpoint, ControlRequest::Shutdown).await {
        Ok(ControlResponse::Accepted { .. }) => {}
        Ok(response) => return Err(CommandError::UnexpectedResponse { response }),
        Err(CommandError::Control(ControlError::EndpointUnavailable)) => {
            println!("WokRouter daemon is already stopped.");
            return Ok(0);
        }
        Err(error) => return Err(error),
    }

    let deadline = Instant::now() + STOP_TIMEOUT;
    loop {
        if Instant::now() >= deadline {
            return Err(CommandError::StopTimedOut);
        }
        match connect_before(&endpoint, operation_deadline(deadline)).await {
            Err(CommandError::Control(ControlError::EndpointUnavailable)) => {
                println!("WokRouter daemon is stopped.");
                return Ok(0);
            }
            Err(CommandError::RequestTimedOut) => {}
            Err(error) => return Err(error),
            Ok(_) => {
                tokio::time::sleep_until(std::cmp::min(deadline, Instant::now() + RETRY_DELAY))
                    .await;
            }
        }
    }
}
