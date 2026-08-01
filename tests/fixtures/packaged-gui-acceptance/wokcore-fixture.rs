use std::{
    env, fs,
    fs::OpenOptions,
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process,
    thread,
    time::{Duration, Instant},
};

const INSTANCE_ID: &str = "13f03bf5-8f5b-4cd5-8f20-73c8570f84b9";
const INSTALLATION_ID: &str =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const TOKEN: &str = "wok_proxy_v1_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const CURRENT_VERSION: &str = "1.0.0";
const TARGET_VERSION: &str = "2.0.0";
const REQUIRED_SCOPES: [&str; 9] = [
    "service.read",
    "service.control",
    "providers.read",
    "providers.write",
    "clients.manage",
    "sessions.read",
    "usage.read",
    "diagnostics.read",
    "diagnostics.export",
];

fn main() {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    let exit = match arguments.as_slice() {
        [command, json] if command == "serve" && json == "--json" => serve(),
        [command, rest @ ..] if command == "authorize" => authorize(rest),
        [command, check, json]
            if command == "update" && check == "--check" && json == "--json" =>
        {
            update_check()
        }
        [command, install, json, progress]
            if command == "update"
                && install == "--install"
                && json == "--json"
                && progress == "--progress-jsonl" =>
        {
            update_install()
        }
        [command, rest @ ..] if command == "feed" => feed(rest),
        _ => 64,
    };
    process::exit(exit);
}

fn state_root() -> Result<PathBuf, ()> {
    let root = env::var_os("WOKROUTER_ACCEPTANCE_STATE_ROOT")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or(())?;
    fs::create_dir_all(&root).map_err(|_| ())?;
    Ok(root)
}

fn log_invocation(root: &Path, value: &str) {
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("fixture.log"))
    {
        let _ = writeln!(file, "pid={} {value}", process::id());
    }
}

fn current_version(root: &Path) -> String {
    fs::read_to_string(root.join("current-version.txt"))
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| CURRENT_VERSION.to_owned())
}

fn serve() -> i32 {
    let Ok(root) = state_root() else {
        return 65;
    };
    log_invocation(&root, "serve");
    let Ok(listener) = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)) else {
        return 66;
    };
    let Ok(address) = listener.local_addr() else {
        return 66;
    };
    let port = address.port();
    if fs::write(root.join("serve-port.txt"), port.to_string()).is_err()
        || fs::write(root.join("serve-pid.txt"), process::id().to_string()).is_err()
        || write_discovery(&root, port, process::id(), &current_version(&root)).is_err()
        || fs::write(root.join("serve-ready"), b"ready").is_err()
    {
        return 67;
    }

    for connection in listener.incoming() {
        let Ok(stream) = connection else {
            continue;
        };
        let root = root.clone();
        thread::spawn(move || handle_wokcore_request(stream, &root));
    }
    0
}

fn write_discovery(root: &Path, port: u16, pid: u32, version: &str) -> Result<(), ()> {
    let local = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or(())?;
    let directory = local.join("WokCore").join("runtime");
    fs::create_dir_all(&directory).map_err(|_| ())?;
    let document = format!(
        "{{\"base_url\":\"http://127.0.0.1:{port}/\",\"pid\":{pid},\"instance_id\":\"{INSTANCE_ID}\",\"wokcore_version\":\"{version}\",\"api_major\":1}}\n"
    );
    let temporary = root.join("discovery.json.tmp");
    fs::write(&temporary, document).map_err(|_| ())?;
    fs::rename(temporary, directory.join("discovery.json")).map_err(|_| ())
}

fn handle_wokcore_request(mut stream: TcpStream, root: &Path) {
    let Ok(request) = read_request(&mut stream) else {
        return;
    };
    let first = request.lines().next().unwrap_or_default();
    let path = first.split_ascii_whitespace().nth(1).unwrap_or_default();
    let authorized = request
        .lines()
        .any(|line| line.eq_ignore_ascii_case(&format!("Authorization: Bearer {TOKEN}")));
    log_invocation(root, &format!("http {path}"));
    let version = current_version(root);
    let (status, body) = match path {
        "/wokcore/v1/health" => (
            "200 OK",
            format!("{{\"status\":\"ok\",\"instance_id\":\"{INSTANCE_ID}\"}}"),
        ),
        "/wokcore/v1/capabilities" => (
            "200 OK",
            format!(
                "{{\"wokcore_version\":\"{version}\",\"management_api_major\":1,\"minimum_management_api_major\":1,\"maximum_management_api_major\":1,\"provider_protocols\":[],\"capabilities\":[\"core.update.v1\"],\"instance_id\":\"{INSTANCE_ID}\",\"installation_id\":\"{INSTALLATION_ID}\"}}"
            ),
        ),
        "/wokcore/v1/service/status" if authorized => (
            "200 OK",
            "{\"phase\":\"running\",\"active_requests\":0}".to_owned(),
        ),
        "/wokcore/v1/service/status" => (
            "401 Unauthorized",
            "{\"error\":\"unauthorized\"}".to_owned(),
        ),
        _ => ("404 Not Found", "{\"error\":\"not_found\"}".to_owned()),
    };
    write_response(&mut stream, status, "application/json", body.as_bytes(), 0);
    log_invocation(root, &format!("http-complete {path}"));
}

fn authorize(arguments: &[String]) -> i32 {
    let Ok(root) = state_root() else {
        return 65;
    };
    log_invocation(&root, "authorize");
    if !valid_authorize_arguments(arguments) {
        return 64;
    }
    let scopes = REQUIRED_SCOPES
        .iter()
        .map(|scope| format!("\"{scope}\""))
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "{{\"client_id\":\"wokrouter.desktop\",\"token_id\":\"acceptance-token\",\"token\":\"{TOKEN}\",\"scopes\":[{scopes}]}}"
    );
    0
}

fn valid_authorize_arguments(arguments: &[String]) -> bool {
    if arguments.len() != 21
        || arguments.first().map(String::as_str) != Some("--client")
        || arguments.get(1).map(String::as_str) != Some("wokrouter.desktop")
        || arguments.last().map(String::as_str) != Some("--json")
    {
        return false;
    }
    let observed = arguments[2..20]
        .chunks_exact(2)
        .filter_map(|pair| (pair[0] == "--scope").then_some(pair[1].as_str()))
        .collect::<Vec<_>>();
    observed == REQUIRED_SCOPES
}

fn update_check() -> i32 {
    let Ok(root) = state_root() else {
        return 65;
    };
    log_invocation(&root, "update-check");
    let current = current_version(&root);
    if current == TARGET_VERSION {
        println!("{{\"code\":\"current\",\"current_version\":\"{current}\"}}");
    } else {
        println!(
            "{{\"code\":\"update_available\",\"current_version\":\"{current}\",\"version\":\"{TARGET_VERSION}\"}}"
        );
    }
    0
}

fn update_install() -> i32 {
    let Ok(root) = state_root() else {
        return 65;
    };
    log_invocation(&root, "update-install");
    let scenario = fs::read_to_string(root.join("scenario.txt"))
        .unwrap_or_else(|_| "success".to_owned());
    match scenario.trim() {
        "active_requests" => update_active_requests(),
        "rollback" => update_rollback(),
        "slow_success" => {
            progress(0, "running", "checking_release", None, None);
            progress(1, "running", "downloading", None, Some((1, 2)));
            if !wait_for_path(&root.join("allow-update"), Duration::from_secs(120)) {
                return 75;
            }
            update_success(&root, 2)
        }
        _ => update_success(&root, 0),
    }
}

fn update_success(root: &Path, start_sequence: u64) -> i32 {
    let phases = [
        "checking_release",
        "downloading",
        "verifying",
        "preparing_service",
        "draining",
        "stopping",
        "installing",
        "starting",
        "verifying_runtime",
    ];
    for (index, phase) in phases.iter().enumerate().skip(start_sequence as usize) {
        let bytes = (*phase == "downloading").then_some((2, 2));
        progress(index as u64, "running", phase, None, bytes);
        thread::sleep(Duration::from_millis(40));
    }
    if fs::write(root.join("current-version.txt"), TARGET_VERSION).is_err() {
        return 74;
    }
    let port = read_number::<u16>(&root.join("serve-port.txt"));
    let pid = read_number::<u32>(&root.join("serve-pid.txt"));
    if let (Some(port), Some(pid)) = (port, pid) {
        let _ = write_discovery(root, port, pid, TARGET_VERSION);
    }
    progress(9, "succeeded", "completed", None, None);
    println!(
        "{{\"code\":\"installed\",\"from\":\"{CURRENT_VERSION}\",\"to\":\"{TARGET_VERSION}\"}}"
    );
    0
}

fn update_active_requests() -> i32 {
    progress(0, "running", "preparing_service", Some(2), None);
    progress(1, "running", "draining", Some(2), None);
    progress_failure(2, "draining", "active_requests_remain", Some(2));
    71
}

fn update_rollback() -> i32 {
    for (sequence, phase) in [
        "checking_release",
        "downloading",
        "verifying",
        "installing",
        "starting",
        "verifying_runtime",
        "rolling_back",
    ]
    .iter()
    .enumerate()
    {
        let bytes = (*phase == "downloading").then_some((4, 8));
        progress(sequence as u64, "running", phase, None, bytes);
    }
    progress_failure(7, "rolling_back", "rolled_back", None);
    72
}

fn progress(
    sequence: u64,
    state: &str,
    phase: &str,
    active_requests: Option<u64>,
    bytes: Option<(u64, u64)>,
) {
    let active = active_requests
        .map(|value| format!(",\"active_requests\":{value}"))
        .unwrap_or_default();
    let bytes = bytes
        .map(|(completed, total)| {
            format!(",\"bytes_completed\":{completed},\"bytes_total\":{total}")
        })
        .unwrap_or_default();
    eprintln!(
        "{{\"schema_version\":1,\"sequence\":{sequence},\"operation\":\"update\",\"state\":\"{state}\",\"phase\":\"{phase}\",\"current_version\":\"{CURRENT_VERSION}\",\"target_version\":\"{TARGET_VERSION}\"{active}{bytes}}}"
    );
    let _ = std::io::stderr().flush();
}

fn progress_failure(
    sequence: u64,
    phase: &str,
    error_code: &str,
    active_requests: Option<u64>,
) {
    let active = active_requests
        .map(|value| format!(",\"active_requests\":{value}"))
        .unwrap_or_default();
    eprintln!(
        "{{\"schema_version\":1,\"sequence\":{sequence},\"operation\":\"update\",\"state\":\"failed\",\"phase\":\"{phase}\",\"current_version\":\"{CURRENT_VERSION}\",\"target_version\":\"{TARGET_VERSION}\"{active},\"error_code\":\"{error_code}\"}}"
    );
    let _ = std::io::stderr().flush();
}

fn feed(arguments: &[String]) -> i32 {
    let Some(root) = argument_value(arguments, "--root").map(PathBuf::from) else {
        return 64;
    };
    let Some(ready) = argument_value(arguments, "--ready").map(PathBuf::from) else {
        return 64;
    };
    let port = argument_value(arguments, "--port")
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(0);
    let Ok(listener) = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)) else {
        return 66;
    };
    let Ok(address) = listener.local_addr() else {
        return 66;
    };
    if fs::write(ready, address.port().to_string()).is_err() {
        return 67;
    }
    if let Ok(state) = state_root() {
        log_invocation(&state, &format!("feed port={}", address.port()));
    }
    for connection in listener.incoming() {
        let Ok(stream) = connection else {
            continue;
        };
        let root = root.clone();
        thread::spawn(move || handle_feed_request(stream, &root));
    }
    0
}

fn handle_feed_request(mut stream: TcpStream, root: &Path) {
    let Ok(request) = read_request(&mut stream) else {
        return;
    };
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_ascii_whitespace().nth(1))
        .unwrap_or_default();
    if let Ok(state) = state_root() {
        log_invocation(&state, &format!("feed-request {path}"));
    }
    let Some(file_name) = path.strip_prefix("/releases/") else {
        write_response(&mut stream, "404 Not Found", "text/plain", b"missing", 0);
        return;
    };
    if file_name.is_empty()
        || file_name.contains('/')
        || file_name.contains('\\')
        || file_name.contains("..")
    {
        write_response(&mut stream, "404 Not Found", "text/plain", b"missing", 0);
        return;
    }
    let path = root.join(file_name);
    let Ok(bytes) = fs::read(path) else {
        write_response(&mut stream, "404 Not Found", "text/plain", b"missing", 0);
        return;
    };
    let delay = if file_name.ends_with(".zip") {
        env::var("WOKROUTER_ACCEPTANCE_FEED_DELAY_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0)
    } else {
        0
    };
    write_response(
        &mut stream,
        "200 OK",
        "application/octet-stream",
        &bytes,
        delay,
    );
}

fn read_request(stream: &mut TcpStream) -> Result<String, ()> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let mut bytes = Vec::with_capacity(2048);
    let mut buffer = [0_u8; 1024];
    while bytes.len() <= 16 * 1024 {
        let read = stream.read(&mut buffer).map_err(|_| ())?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8(bytes).map_err(|_| ())
}

fn write_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
    delay_ms: u64,
) {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    if stream.write_all(header.as_bytes()).is_err() {
        return;
    }
    for chunk in body.chunks(16 * 1024) {
        if stream.write_all(chunk).is_err() || stream.flush().is_err() {
            return;
        }
        if delay_ms > 0 {
            thread::sleep(Duration::from_millis(delay_ms));
        }
    }
}

fn argument_value<'a>(arguments: &'a [String], name: &str) -> Option<&'a str> {
    arguments
        .windows(2)
        .find_map(|pair| (pair[0] == name).then_some(pair[1].as_str()))
}

fn wait_for_path(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.is_file() {
            return true;
        }
        thread::sleep(Duration::from_millis(25));
    }
    false
}

fn read_number<T: std::str::FromStr>(path: &Path) -> Option<T> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}
