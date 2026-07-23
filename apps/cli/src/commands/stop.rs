use std::time::{Duration, Instant};

use wokrouter_control::{ControlClient, ControlError, ControlRequest, ControlResponse};
use wokrouter_platform::AppPaths;

use super::{CommandError, endpoint};

const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY_DELAY: Duration = Duration::from_millis(50);

pub async fn execute(paths: &AppPaths) -> Result<u8, CommandError> {
    let endpoint = endpoint(paths)?;
    let client = match ControlClient::connect(&endpoint).await {
        Ok(client) => client,
        Err(ControlError::EndpointUnavailable) => {
            println!("WokRouter daemon is already stopped.");
            return Ok(0);
        }
        Err(error) => return Err(error.into()),
    };
    match client.request(ControlRequest::Shutdown).await? {
        ControlResponse::Accepted { .. } => {}
        response => return Err(CommandError::UnexpectedResponse { response }),
    }

    let deadline = Instant::now() + STOP_TIMEOUT;
    loop {
        match ControlClient::connect(&endpoint).await {
            Err(ControlError::EndpointUnavailable) => {
                println!("WokRouter daemon is stopped.");
                return Ok(0);
            }
            Err(error) => return Err(error.into()),
            Ok(_) if Instant::now() >= deadline => return Err(CommandError::StopTimedOut),
            Ok(_) => tokio::time::sleep(RETRY_DELAY).await,
        }
    }
}
