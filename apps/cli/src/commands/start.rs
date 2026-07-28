use std::{
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};

use tokio::time::Instant;
use wokrouter_platform::AppPaths;
use wokrouter_wokcore_client::{CoreConnection, ServiceError};

use super::{CommandError, authorize, client, executable, reauthorize};

const START_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY_DELAY: Duration = Duration::from_millis(50);

pub async fn execute(paths: &AppPaths) -> Result<u8, CommandError> {
    let executable = executable(paths)?;
    let client = client(paths)?;
    if let CoreConnection::Running(_) = client.connection().await {
        ensure_authorized(&client, executable).await?;
        println!("WokCore is already running.");
        return Ok(0);
    }

    let mut child = spawn_core(&executable)?;
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        match client.connection().await {
            CoreConnection::Running(_) => {
                ensure_authorized(&client, executable).await?;
                println!("WokCore is running.");
                return Ok(0);
            }
            CoreConnection::Incompatible(_) => {
                kill_created_child(&mut child);
                return Err(CommandError::Incompatible);
            }
            CoreConnection::InvalidRuntime => {
                kill_created_child(&mut child);
                return Err(CommandError::InvalidRuntime);
            }
            CoreConnection::Missing | CoreConnection::Stopped => {}
        }

        if Instant::now() >= deadline {
            let child_exited = child.try_wait().is_ok_and(|status| status.is_some());
            kill_created_child(&mut child);
            return Err(if child_exited {
                CommandError::StartFailed
            } else {
                CommandError::StartTimedOut
            });
        }
        tokio::time::sleep_until(std::cmp::min(deadline, Instant::now() + RETRY_DELAY)).await;
    }
}

async fn ensure_authorized(
    client: &wokrouter_wokcore_client::WokCoreClient,
    executable: std::path::PathBuf,
) -> Result<(), CommandError> {
    let token = authorize(executable.clone()).await?;
    match client.service_status(&token).await {
        Ok(_) => Ok(()),
        Err(ServiceError::Unauthorized | ServiceError::Forbidden) => {
            let token = reauthorize(executable).await?;
            client.service_status(&token).await?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn spawn_core(executable: &Path) -> Result<Child, CommandError> {
    spawn_command(executable)
        .spawn()
        .map_err(|_| CommandError::StartFailed)
}

fn spawn_command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    command
        .args(["serve", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn kill_created_child(child: &mut Child) {
    if child.try_wait().is_ok_and(|status| status.is_none()) {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsStr, path::Path};

    use super::spawn_command;

    #[test]
    fn start_process_contains_only_the_fixed_serve_command() {
        let executable = Path::new(r"C:\Program Files\WokCore\wokcore.exe");
        let command = spawn_command(executable);

        assert_eq!(command.get_program(), executable.as_os_str());
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![OsStr::new("serve"), OsStr::new("--json")]
        );
    }
}
