use std::{
    fs,
    io::{Read, Seek},
    net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream},
    process::{Command, Output, Stdio},
    sync::{Barrier, Mutex},
    thread,
    time::{Duration, Instant},
};

use wokrouter_control::{
    CONTROL_PROTOCOL_VERSION, ControlClient, ControlEndpoint, ControlError, ControlRequest,
    ControlResponse, DaemonState, DaemonStatus,
};
use wokrouter_daemon::DaemonRuntime;
use wokrouter_platform::AppPaths;
use wokrouter_storage::{AppConfig, ConfigStore};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const DAEMON_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

static TEST_LOCK: Mutex<()> = Mutex::new(());
#[test]
fn cli_starts_reports_and_stops_daemon() {
    if daemon_helper_mode() {
        return;
    }
    let _serial = TEST_LOCK.lock().unwrap();
    let home = TestHome::with_persisted_free_port();

    assert_success(wokrouter(&home, &["start"]));

    let human = assert_success(wokrouter(&home, &["status"]));
    assert!(stdout(&human).contains("running"));
    assert!(stdout(&human).contains(env!("CARGO_PKG_VERSION")));

    let json = assert_success(wokrouter(&home, &["status", "--json"]));
    assert_eq!(
        stdout(&json),
        format!(
            "{{\"state\":\"running\",\"version\":\"{}\"}}\n",
            env!("CARGO_PKG_VERSION")
        )
    );

    assert_success(wokrouter(&home, &["stop"]));

    let human = wokrouter(&home, &["status"]);
    assert_eq!(human.status.code(), Some(3));
    assert!(stdout(&human).contains("stopped"));

    let json = wokrouter(&home, &["status", "--json"]);
    assert_eq!(json.status.code(), Some(3));
    assert_eq!(
        stdout(&json),
        format!(
            "{{\"state\":\"stopped\",\"version\":\"{}\"}}\n",
            env!("CARGO_PKG_VERSION")
        )
    );
}

#[test]
fn duplicate_concurrent_start_returns_success_with_one_daemon_pid() {
    if daemon_helper_mode() {
        return;
    }
    let _serial = TEST_LOCK.lock().unwrap();
    let home = TestHome::with_persisted_free_port();
    let barrier = Barrier::new(2);

    thread::scope(|scope| {
        let first = scope.spawn(|| {
            barrier.wait();
            wokrouter(&home, &["start"])
        });
        let second = scope.spawn(|| {
            barrier.wait();
            wokrouter(&home, &["start"])
        });

        assert_success(first.join().unwrap());
        assert_success(second.join().unwrap());
    });

    let first_pid = home.daemon_pid();
    assert_ne!(first_pid, std::process::id());
    assert_success(wokrouter(&home, &["status"]));
    assert_eq!(home.daemon_pid(), first_pid);
    assert_success(wokrouter(&home, &["stop"]));
}

#[test]
fn first_start_persists_one_free_fallback_port_and_reuses_it() {
    if daemon_helper_mode() {
        return;
    }
    let _serial = TEST_LOCK.lock().unwrap();
    let occupied_default = TcpListener::bind((Ipv4Addr::LOCALHOST, 10101)).unwrap();
    let home = TestHome::new();

    assert_success(wokrouter(&home, &["start"]));
    let first = home.committed_config();
    assert_ne!(first.config.server.port, 10101);
    assert_eq!(first.revision, 1);
    assert!(
        TcpStream::connect_timeout(
            &SocketAddrV4::new(Ipv4Addr::LOCALHOST, first.config.server.port).into(),
            Duration::from_secs(1),
        )
        .is_ok()
    );

    assert_success(wokrouter(&home, &["stop"]));
    assert_success(wokrouter(&home, &["start"]));
    let second = home.committed_config();
    assert_eq!(second.config.server.port, first.config.server.port);
    assert_eq!(second.revision, first.revision);
    assert_success(wokrouter(&home, &["stop"]));
    drop(occupied_default);
}

#[test]
fn persisted_port_conflict_is_typed_and_does_not_drift_config() {
    if daemon_helper_mode() {
        return;
    }
    let _serial = TEST_LOCK.lock().unwrap();
    let occupied = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = occupied.local_addr().unwrap().port();
    let home = TestHome::with_persisted_port(port);
    let before = home.committed_config();

    let start = wokrouter(&home, &["start"]);
    assert!(!start.status.success());
    assert!(stderr(&start).contains("configured data-plane port"));
    assert!(stderr(&start).contains(&port.to_string()));
    assert_eq!(home.committed_config(), before);

    drop(occupied);
}

#[test]
fn reload_requires_the_committed_revision_and_shutdown_closes_ipc() {
    if daemon_helper_mode() {
        return;
    }
    let _serial = TEST_LOCK.lock().unwrap();
    let home = TestHome::with_persisted_free_port();
    assert_success(wokrouter(&home, &["start"]));

    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let endpoint = ControlEndpoint::for_runtime_dir(&home.paths.runtime_dir).unwrap();
        let client = ControlClient::connect(&endpoint).await.unwrap();

        assert_eq!(
            client.request(ControlRequest::Ping).await.unwrap(),
            ControlResponse::Pong {
                protocol_version: CONTROL_PROTOCOL_VERSION,
            }
        );
        assert_eq!(
            client.request(ControlRequest::Status).await.unwrap(),
            ControlResponse::Status(DaemonStatus {
                state: DaemonState::Running,
                version: env!("CARGO_PKG_VERSION").to_owned(),
            })
        );

        let active = home.committed_config();
        let expected = active.revision + 1;
        let error = client
            .request(ControlRequest::Reload {
                expected_revision: expected,
            })
            .await
            .unwrap_err();
        assert_eq!(
            error,
            ControlError::RevisionConflict {
                expected,
                actual: active.revision,
            }
        );

        let committed = ConfigStore::new(&home.paths.config_file)
            .commit(active.revision, &active.config)
            .unwrap();
        assert_eq!(
            client
                .request(ControlRequest::Reload {
                    expected_revision: committed.revision,
                })
                .await
                .unwrap(),
            ControlResponse::Accepted {
                revision: committed.revision,
            }
        );
        assert_eq!(
            client.request(ControlRequest::Shutdown).await.unwrap(),
            ControlResponse::Accepted {
                revision: committed.revision,
            }
        );
        let deadline = tokio::time::Instant::now() + DAEMON_CLOSE_TIMEOUT;
        loop {
            if ControlClient::connect(&endpoint).await.is_err() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "control endpoint did not close within {DAEMON_CLOSE_TIMEOUT:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });

    let status = wokrouter(&home, &["status"]);
    assert_eq!(status.status.code(), Some(3));
}

#[test]
fn daemon_process_helper() {
    if !daemon_helper_mode() {
        return;
    }
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let paths = AppPaths::discover().unwrap();
        let daemon = DaemonRuntime::start(paths)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        daemon.run_until_shutdown().await.unwrap();
    });
}

struct TestHome {
    directory: tempfile::TempDir,
    paths: AppPaths,
}

impl TestHome {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let paths = AppPaths {
            config_file: directory
                .path()
                .join("config")
                .join("WokRouter")
                .join("config.toml"),
            state_db: directory
                .path()
                .join("state")
                .join("WokRouter")
                .join("state.sqlite3"),
            runtime_dir: directory
                .path()
                .join("state")
                .join("WokRouter")
                .join("runtime"),
            log_dir: directory
                .path()
                .join("state")
                .join("WokRouter")
                .join("logs"),
        };
        Self { directory, paths }
    }

    fn with_persisted_free_port() -> Self {
        let probe = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        Self::with_persisted_port(port)
    }

    fn with_persisted_port(port: u16) -> Self {
        let home = Self::new();
        fs::create_dir_all(home.paths.config_file.parent().unwrap()).unwrap();
        let mut config = AppConfig::default();
        config.server.port = port;
        ConfigStore::new(&home.paths.config_file)
            .commit(0, &config)
            .unwrap();
        home
    }

    fn committed_config(&self) -> wokrouter_storage::VersionedConfig {
        ConfigStore::new(&self.paths.config_file).load().unwrap()
    }

    fn daemon_pid(&self) -> u32 {
        fs::read_to_string(self.paths.runtime_dir.join("wokrouterd.pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }
}

impl Drop for TestHome {
    fn drop(&mut self) {
        let pid_path = self.paths.runtime_dir.join("wokrouterd.pid");
        if !pid_path.exists() {
            return;
        }

        cleanup_command(wokrouter_command(self, &["stop"]), COMMAND_TIMEOUT);
        if !pid_path.exists() {
            return;
        }

        let Ok(pid) = fs::read_to_string(&pid_path).map(|pid| pid.trim().to_owned()) else {
            return;
        };
        #[cfg(windows)]
        let _ = Command::new("taskkill")
            .args(["/PID", &pid, "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        #[cfg(unix)]
        let _ = Command::new("kill")
            .args(["-TERM", &pid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn wokrouter(home: &TestHome, arguments: &[&str]) -> Output {
    run_command(wokrouter_command(home, arguments), COMMAND_TIMEOUT)
}

fn wokrouter_command(home: &TestHome, arguments: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_wokrouter"));
    command
        .args(arguments)
        .env("APPDATA", home.directory.path().join("config"))
        .env("LOCALAPPDATA", home.directory.path().join("state"))
        .env("USERPROFILE", home.directory.path())
        .env("HOME", home.directory.path())
        .env("WOKROUTER_DAEMON_EXE", std::env::current_exe().unwrap())
        .env(
            "WOKROUTER_DAEMON_ARGS",
            "--exact daemon_process_helper --nocapture --test-threads=1",
        )
        .env("WOKROUTER_TEST_DAEMON", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn run_command(mut command: Command, timeout: Duration) -> Output {
    let mut stdout_file = tempfile::tempfile().unwrap();
    let mut stderr_file = tempfile::tempfile().unwrap();
    command
        .stdout(stdout_file.try_clone().unwrap())
        .stderr(stderr_file.try_clone().unwrap());
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return Output {
                status,
                stdout: read_file(&mut stdout_file),
                stderr: read_file(&mut stderr_file),
            };
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let status = child.wait().unwrap();
            let output = Output {
                status,
                stdout: read_file(&mut stdout_file),
                stderr: read_file(&mut stderr_file),
            };
            panic!(
                "command timed out after {timeout:?}; stdout={}; stderr={}",
                stdout(&output),
                stderr(&output),
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn read_file(file: &mut fs::File) -> Vec<u8> {
    file.rewind().unwrap();
    let mut contents = Vec::new();
    file.read_to_end(&mut contents).unwrap();
    contents
}

fn assert_success(output: Output) -> Output {
    assert!(
        output.status.success(),
        "command failed with {:?}; stdout={}; stderr={}",
        output.status.code(),
        stdout(&output),
        stderr(&output),
    );
    output
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn cleanup_command(mut command: Command, timeout: Duration) {
    let Ok(mut child) = command.spawn() else {
        return;
    };
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let _ = child.wait();
                return;
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
        }
    }
}

fn daemon_helper_mode() -> bool {
    std::env::var_os("WOKROUTER_TEST_DAEMON").is_some()
}
