use std::process::ExitCode;

use wokrouter_cli::commands::{CommandError, start};
use wokrouter_platform::{AppPaths, select_wokcore_runtime};

#[tokio::main]
async fn main() -> ExitCode {
    match run(std::env::args().skip(1)).await {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("wokrouter: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(arguments: impl Iterator<Item = String>) -> Result<u8, CommandError> {
    parse_arguments(arguments)?;
    let mut output = start::StandardStartCommandOutput;
    let paths = match AppPaths::discover() {
        Ok(paths) => paths,
        Err(error) => return Ok(start::render_structured_platform_error(error, &mut output)),
    };
    let runtime = match select_wokcore_runtime(&paths).await {
        Ok(runtime) => runtime,
        Err(error) => return Ok(start::render_structured_platform_error(error, &mut output)),
    };
    start::execute_packaged_acceptance_with_options(
        &paths,
        &runtime,
        start::StartOptions {
            json: true,
            progress_jsonl: true,
        },
        &mut output,
    )
    .await
}

fn parse_arguments(arguments: impl Iterator<Item = String>) -> Result<(), CommandError> {
    if arguments.collect::<Vec<_>>() == ["start", "--json", "--progress-jsonl"] {
        Ok(())
    } else {
        Err(CommandError::Usage)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_arguments;
    use wokrouter_cli::commands::CommandError;

    #[test]
    fn only_structured_start_is_accepted() {
        assert_eq!(parse(&["start", "--json", "--progress-jsonl"]), Ok(()));
        assert_eq!(parse(&["start"]), Err(CommandError::Usage));
        assert_eq!(
            parse(&["start", "--json", "--progress-jsonl", "extra"]),
            Err(CommandError::Usage)
        );
    }

    fn parse(arguments: &[&str]) -> Result<(), CommandError> {
        parse_arguments(arguments.iter().map(|value| (*value).to_owned()))
    }
}
