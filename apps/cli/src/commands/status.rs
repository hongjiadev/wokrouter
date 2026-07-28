use wokrouter_platform::AppPaths;
use wokrouter_wokcore_client::{CoreConnection, ServiceError};

use super::{
    AUTHORIZATION_REQUIRED_EXIT_CODE, CommandError, CoreStatus, CoreUiState, NOT_RUNNING_EXIT_CODE,
    client, executable, load_token, protected_status, public_status,
};

pub async fn execute(paths: &AppPaths, json: bool) -> Result<u8, CommandError> {
    let (status, exit_code) = snapshot(paths).await?;
    render(&status, json);
    Ok(exit_code)
}

pub async fn snapshot(paths: &AppPaths) -> Result<(CoreStatus, u8), CommandError> {
    match executable(paths) {
        Ok(_) => {}
        Err(CommandError::WokCoreMissing) => {
            let status = CoreStatus::missing();
            return Ok((status, NOT_RUNNING_EXIT_CODE));
        }
        Err(error) => return Err(error),
    }
    let client = client(paths)?;
    let connection = client.connection().await;
    let (status, exit_code) = match connection {
        CoreConnection::Running(handshake) => match load_token().await? {
            None => (
                public_status(CoreConnection::Running(handshake)),
                AUTHORIZATION_REQUIRED_EXIT_CODE,
            ),
            Some(token) => match client.service_status(&token).await {
                Ok(service) => (protected_status(handshake, service), 0),
                Err(ServiceError::Unauthorized | ServiceError::Forbidden) => (
                    public_status(CoreConnection::Running(handshake)),
                    AUTHORIZATION_REQUIRED_EXIT_CODE,
                ),
                Err(error) => return Err(error.into()),
            },
        },
        other => {
            let status = public_status(other);
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
