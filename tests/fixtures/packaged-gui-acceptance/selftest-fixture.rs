use std::{
    env,
    fs::{self, OpenOptions},
    path::PathBuf,
    process,
    thread,
    time::Duration,
};

#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;

#[derive(Default)]
struct Options {
    marker: Option<PathBuf>,
    lease: Option<PathBuf>,
    ready: Option<PathBuf>,
    release: Option<PathBuf>,
    sleep_ms: u64,
    exit_code: i32,
}

fn main() {
    let options = parse_options().unwrap_or_else(|| process::exit(64));
    let _lease = match options.lease.as_ref() {
        Some(path) => match open_exclusive(path) {
            Ok(file) => Some(file),
            Err(_) => process::exit(73),
        },
        None => None,
    };

    if let Some(path) = options.marker.as_ref() {
        fs::write(path, process::id().to_string()).unwrap_or_else(|_| process::exit(74));
    }
    if let Some(path) = options.ready.as_ref() {
        fs::write(path, b"ready").unwrap_or_else(|_| process::exit(74));
    }
    if let Some(path) = options.release.as_ref() {
        while !path.exists() {
            thread::sleep(Duration::from_millis(10));
        }
    }
    if options.sleep_ms != 0 {
        thread::sleep(Duration::from_millis(options.sleep_ms));
    }
    process::exit(options.exit_code);
}

fn parse_options() -> Option<Options> {
    let mut options = Options::default();
    let mut arguments = env::args_os().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.to_str()? {
            "--marker" => options.marker = Some(arguments.next()?.into()),
            "--lease" => options.lease = Some(arguments.next()?.into()),
            "--ready" => options.ready = Some(arguments.next()?.into()),
            "--release" => options.release = Some(arguments.next()?.into()),
            "--sleep-ms" => options.sleep_ms = arguments.next()?.to_str()?.parse().ok()?,
            "--exit-code" => options.exit_code = arguments.next()?.to_str()?.parse().ok()?,
            _ => return None,
        }
    }
    Some(options)
}

#[cfg(windows)]
fn open_exclusive(path: &PathBuf) -> std::io::Result<std::fs::File> {
    OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .share_mode(0)
        .open(path)
}

#[cfg(not(windows))]
fn open_exclusive(path: &PathBuf) -> std::io::Result<std::fs::File> {
    OpenOptions::new().create_new(true).write(true).open(path)
}
