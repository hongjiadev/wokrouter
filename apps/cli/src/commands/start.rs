use std::{
    ffi::OsString,
    io::Read,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use wokrouter_control::ControlError;
use wokrouter_platform::AppPaths;

use super::{CommandError, endpoint, ping};

const START_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY_DELAY: Duration = Duration::from_millis(50);

pub async fn execute(paths: &AppPaths) -> Result<u8, CommandError> {
    let endpoint = endpoint(paths)?;
    match ping(&endpoint).await {
        Ok(()) => {
            println!("WokRouter daemon is already running.");
            return Ok(0);
        }
        Err(CommandError::Control(ControlError::EndpointUnavailable)) => {}
        Err(error) => return Err(error),
    }

    let mut child = spawn_daemon()?;
    let deadline = Instant::now() + START_TIMEOUT;
    let mut startup_error = None;
    loop {
        match ping(&endpoint).await {
            Ok(()) => {
                println!("WokRouter daemon is running.");
                return Ok(0);
            }
            Err(CommandError::Control(ControlError::EndpointUnavailable))
            | Err(CommandError::RequestTimedOut) => {}
            Err(error) => return Err(error),
        }

        if startup_error.is_none() && child.try_wait()?.is_some() {
            startup_error = read_stderr(&mut child)?;
        }
        if Instant::now() >= deadline {
            if child.try_wait()?.is_none() {
                child.kill()?;
                child.wait()?;
                startup_error = read_stderr(&mut child)?;
            }
            return startup_error
                .filter(|message| !message.is_empty())
                .map(|message| Err(CommandError::DaemonFailed { message }))
                .unwrap_or(Err(CommandError::StartTimedOut));
        }
        tokio::time::sleep(RETRY_DELAY).await;
    }
}

fn spawn_daemon() -> Result<Child, CommandError> {
    let mut command = Command::new(daemon_executable()?);
    if let Some(arguments) = std::env::var_os("WOKROUTER_DAEMON_ARGS") {
        command.args(arguments.to_string_lossy().split_ascii_whitespace());
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    Ok(command.spawn()?)
}

fn daemon_executable() -> Result<OsString, CommandError> {
    if let Some(path) = std::env::var_os("WOKROUTER_DAEMON_EXE") {
        return Ok(path);
    }
    let current = std::env::current_exe()?;
    let file_name = format!("wokrouterd{}", std::env::consts::EXE_SUFFIX);
    Ok(current.with_file_name(file_name).into_os_string())
}

fn read_stderr(child: &mut Child) -> Result<Option<String>, CommandError> {
    let Some(mut stderr) = child.stderr.take() else {
        return Ok(None);
    };
    let mut message = String::new();
    stderr.read_to_string(&mut message)?;
    Ok(Some(message.trim().to_owned()))
}
