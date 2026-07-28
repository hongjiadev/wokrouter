use std::process::ExitCode;

use wokrouter_cli::commands::{self, CommandError};
use wokrouter_platform::AppPaths;

enum Command {
    Start,
    Status { json: bool },
    Stop,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("wokrouter: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<u8, CommandError> {
    let command = parse_command(std::env::args().skip(1))?;
    let paths = AppPaths::discover()?;
    match command {
        Command::Start => commands::start::execute(&paths).await,
        Command::Status { json } => commands::status::execute(&paths, json).await,
        Command::Stop => commands::stop::execute(&paths).await,
    }
}

fn parse_command(mut arguments: impl Iterator<Item = String>) -> Result<Command, CommandError> {
    match (arguments.next().as_deref(), arguments.next().as_deref()) {
        (Some("start"), None) => Ok(Command::Start),
        (Some("status"), None) => Ok(Command::Status { json: false }),
        (Some("status"), Some("--json")) if arguments.next().is_none() => {
            Ok(Command::Status { json: true })
        }
        (Some("stop"), None) => Ok(Command::Stop),
        _ => Err(CommandError::Usage),
    }
}
