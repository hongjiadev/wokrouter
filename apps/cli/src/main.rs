use std::process::ExitCode;

use wokrouter_cli::commands::{self, CommandError};
use wokrouter_platform::{AppPaths, ClientKind, select_wokcore_runtime};

#[derive(Debug, Eq, PartialEq)]
enum Command {
    Start,
    Status { json: bool },
    Stop,
    Integrate { client: ClientKind },
    Restore { client: ClientKind },
    Doctor { json: bool },
    DoctorRepair { check_id: String },
    IntegrationToken { client: ClientKind },
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
        Command::Start => {
            let runtime = select_wokcore_runtime(&paths).await?;
            commands::start::execute(&runtime).await
        }
        Command::Status { json } => {
            let runtime = select_wokcore_runtime(&paths).await?;
            commands::status::execute(&runtime, json).await
        }
        Command::Stop => {
            let runtime = select_wokcore_runtime(&paths).await?;
            commands::stop::execute(&runtime).await
        }
        Command::Integrate { client } => commands::integrations::integrate(&paths, client).await,
        Command::Restore { client } => commands::integrations::restore(&paths, client).await,
        Command::Doctor { json } => commands::integrations::doctor(&paths, json).await,
        Command::DoctorRepair { check_id } => {
            commands::integrations::repair(&paths, &check_id).await
        }
        Command::IntegrationToken { client } => {
            commands::integrations::integration_token(&paths, client)
        }
    }
}

fn parse_command(arguments: impl Iterator<Item = String>) -> Result<Command, CommandError> {
    let arguments = arguments.collect::<Vec<_>>();
    match arguments.as_slice() {
        [command] if command == "start" => Ok(Command::Start),
        [command] if command == "status" => Ok(Command::Status { json: false }),
        [command, option] if command == "status" && option == "--json" => {
            Ok(Command::Status { json: true })
        }
        [command] if command == "stop" => Ok(Command::Stop),
        [command, client] if command == "integrate" => Ok(Command::Integrate {
            client: parse_client(client)?,
        }),
        [command, client] if command == "restore" => Ok(Command::Restore {
            client: parse_client(client)?,
        }),
        [command] if command == "doctor" => Ok(Command::Doctor { json: false }),
        [command, option] if command == "doctor" && option == "--json" => {
            Ok(Command::Doctor { json: true })
        }
        [command, option, check_id]
            if command == "doctor"
                && option == "--repair"
                && commands::integrations::repair_client(check_id).is_some() =>
        {
            Ok(Command::DoctorRepair {
                check_id: check_id.to_owned(),
            })
        }
        [command, client] if command == "integration-token" => Ok(Command::IntegrationToken {
            client: parse_client(client)?,
        }),
        _ => Err(CommandError::Usage),
    }
}

fn parse_client(value: &str) -> Result<ClientKind, CommandError> {
    match value {
        "codex" => Ok(ClientKind::Codex),
        "claude" => Ok(ClientKind::Claude),
        "copilot" => Ok(ClientKind::Copilot),
        _ => Err(CommandError::Usage),
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, parse_command};
    use wokrouter_cli::commands::CommandError;
    use wokrouter_platform::ClientKind;

    #[test]
    fn client_and_doctor_commands_are_parsed_exactly() {
        assert_eq!(
            parse(&["integrate", "codex"]).unwrap(),
            Command::Integrate {
                client: ClientKind::Codex
            }
        );
        assert_eq!(
            parse(&["restore", "claude"]).unwrap(),
            Command::Restore {
                client: ClientKind::Claude
            }
        );
        assert_eq!(
            parse(&["doctor", "--json"]).unwrap(),
            Command::Doctor { json: true }
        );
        assert_eq!(
            parse(&["doctor", "--repair", "copilot_token"]).unwrap(),
            Command::DoctorRepair {
                check_id: "copilot_token".to_owned()
            }
        );
        assert_eq!(
            parse(&["integration-token", "copilot"]).unwrap(),
            Command::IntegrationToken {
                client: ClientKind::Copilot
            }
        );
    }

    #[test]
    fn unknown_clients_checks_and_extra_arguments_are_rejected() {
        assert_eq!(
            parse(&["integrate", "gemini"]).unwrap_err(),
            CommandError::Usage
        );
        assert_eq!(
            parse(&["doctor", "--repair", "all"]).unwrap_err(),
            CommandError::Usage
        );
        assert_eq!(
            parse(&["integration-token", "codex", "extra"]).unwrap_err(),
            CommandError::Usage
        );
    }

    fn parse(arguments: &[&str]) -> Result<Command, CommandError> {
        parse_command(arguments.iter().map(|value| (*value).to_owned()))
    }
}
