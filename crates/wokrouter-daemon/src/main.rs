use std::process::ExitCode;

use wokrouter_daemon::DaemonRuntime;
use wokrouter_platform::AppPaths;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("wokrouterd: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let paths = AppPaths::discover()?;
    let daemon = DaemonRuntime::start(paths).await?;
    daemon.run_until_shutdown().await?;
    Ok(())
}
