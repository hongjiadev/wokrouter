use std::time::Duration;

use tokio::time::Instant;
use wokrouter_platform::AppPaths;
use wokrouter_wokcore_client::{CoreConnection, ServiceError};

use super::{CommandError, authorize, client, executable, reauthorize};

const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY_DELAY: Duration = Duration::from_millis(50);

pub async fn execute(paths: &AppPaths) -> Result<u8, CommandError> {
    let executable = match executable(paths) {
        Ok(executable) => executable,
        Err(CommandError::WokCoreMissing) => {
            println!("WokCore is already stopped.");
            return Ok(0);
        }
        Err(error) => return Err(error),
    };
    let client = client(paths)?;
    match client.connection().await {
        CoreConnection::Missing | CoreConnection::Stopped => {
            println!("WokCore is already stopped.");
            return Ok(0);
        }
        CoreConnection::Incompatible(_) => return Err(CommandError::Incompatible),
        CoreConnection::InvalidRuntime => return Err(CommandError::InvalidRuntime),
        CoreConnection::Running(_) => {}
    }

    let token = authorize(executable.clone()).await?;
    match client.stop(&token).await {
        Ok(()) => {}
        Err(ServiceError::Unauthorized | ServiceError::Forbidden) => {
            let token = reauthorize(executable).await?;
            client.stop(&token).await?;
        }
        Err(error) => return Err(error.into()),
    }

    let deadline = Instant::now() + STOP_TIMEOUT;
    loop {
        match client.connection().await {
            CoreConnection::Missing | CoreConnection::Stopped => {
                println!("WokCore is stopped.");
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
