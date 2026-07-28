use std::{env, fs::OpenOptions, io::Write, process};

fn main() {
    let mut arguments = env::args().skip(1);
    let mut observed_host = None;
    let mut exit_code = None;
    while let Some(argument) = arguments.next() {
        match (argument.as_str(), arguments.next()) {
            ("--observed-host", Some(path)) => observed_host = Some(path),
            ("--exit-code", Some(code)) => match code.parse::<i32>() {
                Ok(code) => exit_code = Some(code),
                Err(_) => process::exit(2),
            },
            _ => process::exit(2),
        }
    }

    if let Some(observed_host) = observed_host {
        let executable = match env::current_exe() {
            Ok(executable) => executable,
            Err(_) => process::exit(2),
        };
        let mut observed = match OpenOptions::new()
            .create(true)
            .append(true)
            .open(observed_host)
        {
            Ok(file) => file,
            Err(_) => process::exit(2),
        };
        if writeln!(observed, "{}", executable.display()).is_err() {
            process::exit(2);
        }
    }

    process::exit(exit_code.unwrap_or(0));
}
