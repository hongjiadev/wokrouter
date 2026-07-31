use std::{
    io::{self, Write},
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};

use async_trait::async_trait;
use secrecy::SecretString;
use tokio::time::Instant;
use wokrouter_platform::{
    AppPaths, PlatformError, SelectedWokCoreRuntime, WokCoreInstallError, WokCoreInstallOutcome,
    WokCoreInstallSource, WokCoreRuntimeChannel, install_missing_wokcore_with_progress,
};
use wokrouter_wokcore_client::{
    CoreConnection, ServiceError, ServicePhase, ServiceStatus, WokCoreClient,
};

use super::{CommandError, CommandRuntime, authorize, reauthorize};

mod progress;

use progress::StartProgressReporter;

const START_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY_DELAY: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartOptions {
    pub json: bool,
    pub progress_jsonl: bool,
}

pub async fn execute(runtime: &SelectedWokCoreRuntime) -> Result<u8, CommandError> {
    let install_source =
        WokCoreInstallSource::production().map_err(|_| CommandError::CoreControl)?;
    let mut output = StandardStartCommandOutput;
    execute_with_runtime_paths(
        None,
        runtime,
        StartOptions {
            json: false,
            progress_jsonl: false,
        },
        &mut output,
        &StartDependencies {
            install_source,
            service: Box::new(SystemStartService),
        },
    )
    .await
}

pub trait StartCommandOutput: Send {
    fn stdout(&mut self, value: &str) -> io::Result<()>;
    fn stderr(&mut self, value: &str) -> io::Result<()>;
}

pub struct StandardStartCommandOutput;

impl StartCommandOutput for StandardStartCommandOutput {
    fn stdout(&mut self, value: &str) -> io::Result<()> {
        io::stdout().write_all(value.as_bytes())
    }

    fn stderr(&mut self, value: &str) -> io::Result<()> {
        io::stderr().write_all(value.as_bytes())
    }
}

pub fn render_structured_platform_error(
    error: PlatformError,
    output: &mut dyn StartCommandOutput,
) -> u8 {
    let code = match error {
        PlatformError::InvalidWokCoreInstallRecord => "invalid_install_state",
        PlatformError::MissingPlatformData { .. } | PlatformError::WokCoreClientInitialization => {
            "start_failed"
        }
    };
    let mut reporter = StartProgressReporter::new(output, true);
    reporter.failed("checking_release", code);
    reporter.stdout_code(code);
    1
}

#[async_trait]
trait StartService: Send + Sync {
    async fn connection(&self, client: &WokCoreClient) -> Result<CoreConnection, CommandError>;
    fn spawn(&self, executable: &Path) -> Result<Box<dyn StartedCore>, CommandError>;
    async fn authorize(
        &self,
        client: &WokCoreClient,
        executable: &Path,
    ) -> Result<SecretString, CommandError>;
    async fn reauthorize(
        &self,
        client: &WokCoreClient,
        executable: &Path,
    ) -> Result<SecretString, CommandError>;
    async fn authorization_status(
        &self,
        client: &WokCoreClient,
        token: &SecretString,
    ) -> Result<ServiceStatus, ServiceError>;
    async fn authenticated_status(
        &self,
        client: &WokCoreClient,
        token: &SecretString,
    ) -> Result<ServiceStatus, ServiceError>;
}

trait StartedCore: Send {
    fn try_wait(&mut self) -> Result<bool, CommandError>;
    fn kill(&mut self) -> Result<(), CommandError>;
    fn wait(&mut self) -> Result<(), CommandError>;
}

struct StartDependencies {
    install_source: WokCoreInstallSource,
    service: Box<dyn StartService>,
}

struct SystemStartService;

#[async_trait]
impl StartService for SystemStartService {
    async fn connection(&self, client: &WokCoreClient) -> Result<CoreConnection, CommandError> {
        Ok(client.connection().await)
    }

    fn spawn(&self, executable: &Path) -> Result<Box<dyn StartedCore>, CommandError> {
        Ok(Box::new(SystemStartedCore(spawn_core(executable)?)))
    }

    async fn authorize(
        &self,
        _client: &WokCoreClient,
        executable: &Path,
    ) -> Result<SecretString, CommandError> {
        authorize(executable.to_path_buf()).await
    }

    async fn reauthorize(
        &self,
        _client: &WokCoreClient,
        executable: &Path,
    ) -> Result<SecretString, CommandError> {
        reauthorize(executable.to_path_buf()).await
    }

    async fn authorization_status(
        &self,
        client: &WokCoreClient,
        token: &SecretString,
    ) -> Result<ServiceStatus, ServiceError> {
        client.service_status(token).await
    }

    async fn authenticated_status(
        &self,
        client: &WokCoreClient,
        token: &SecretString,
    ) -> Result<ServiceStatus, ServiceError> {
        client.service_status(token).await
    }
}

struct SystemStartedCore(Child);

impl StartedCore for SystemStartedCore {
    fn try_wait(&mut self) -> Result<bool, CommandError> {
        self.0
            .try_wait()
            .map(|status| status.is_some())
            .map_err(|_| CommandError::StartFailed)
    }

    fn kill(&mut self) -> Result<(), CommandError> {
        self.0.kill().map_err(|_| CommandError::StartFailed)
    }

    fn wait(&mut self) -> Result<(), CommandError> {
        self.0
            .wait()
            .map(|_| ())
            .map_err(|_| CommandError::StartFailed)
    }
}

pub async fn execute_with_options(
    paths: &AppPaths,
    runtime: &SelectedWokCoreRuntime,
    options: StartOptions,
    output: &mut dyn StartCommandOutput,
) -> Result<u8, CommandError> {
    let structured = validate_options(options)?;
    let install_source = match WokCoreInstallSource::production() {
        Ok(source) => source,
        Err(error) if structured => {
            let code = install_error_code(error);
            let mut reporter = StartProgressReporter::new(output, true);
            reporter.failed("checking_release", code);
            reporter.stdout_code(code);
            return Ok(1);
        }
        Err(_) => return Err(CommandError::CoreControl),
    };
    execute_with_dependencies(
        paths,
        runtime,
        options,
        output,
        &StartDependencies {
            install_source,
            service: Box::new(SystemStartService),
        },
    )
    .await
}

async fn execute_with_dependencies(
    paths: &AppPaths,
    runtime: &(impl CommandRuntime + Sync),
    options: StartOptions,
    output: &mut dyn StartCommandOutput,
    dependencies: &StartDependencies,
) -> Result<u8, CommandError> {
    execute_with_runtime_paths(Some(paths), runtime, options, output, dependencies).await
}

async fn execute_with_runtime_paths(
    paths: Option<&AppPaths>,
    runtime: &(impl CommandRuntime + Sync),
    options: StartOptions,
    output: &mut dyn StartCommandOutput,
    dependencies: &StartDependencies,
) -> Result<u8, CommandError> {
    let structured = validate_options(options)?;
    let mut reporter = StartProgressReporter::new(output, structured);
    match run_start_workflow(paths, runtime, dependencies, &mut reporter).await {
        Ok(outcome) => {
            if structured {
                reporter.completed();
                reporter.stdout_code(if outcome.already_running {
                    "already_running"
                } else {
                    "running"
                });
            } else {
                reporter
                    .human_message(start_message(outcome.already_running))
                    .map_err(|_| CommandError::CoreControl)?;
            }
            Ok(0)
        }
        Err(failure) if structured => {
            reporter.failed(failure.phase, failure.code);
            reporter.stdout_code(failure.code);
            Ok(1)
        }
        Err(failure) => Err(failure.command),
    }
}

async fn run_start_workflow(
    paths: Option<&AppPaths>,
    runtime: &(impl CommandRuntime + Sync),
    dependencies: &StartDependencies,
    reporter: &mut StartProgressReporter<'_>,
) -> Result<StartOutcome, StartFailure> {
    let initial_connection = dependencies
        .service
        .connection(runtime.client())
        .await
        .map_err(|error| StartFailure::start(error, "verifying_runtime"))?;

    if runtime.channel() == WokCoreRuntimeChannel::Development {
        return run_development_start(runtime, dependencies, reporter, initial_connection).await;
    }

    let executable = match runtime.executable() {
        Some(executable) => executable.to_path_buf(),
        None => {
            let paths = paths.ok_or_else(|| {
                StartFailure::start(CommandError::WokCoreMissing, "checking_release")
            })?;
            let outcome = install_missing_wokcore_with_progress(
                paths,
                &dependencies.install_source,
                reporter,
            )
            .await
            .map_err(|error| StartFailure::install(error, reporter.phase()))?;
            match outcome {
                WokCoreInstallOutcome::Installed { executable, .. }
                | WokCoreInstallOutcome::AlreadyInstalled { executable } => executable,
            }
        }
    };

    let already_running = matches!(initial_connection, CoreConnection::Running(_));
    let mut started = None;
    if !already_running {
        match initial_connection {
            CoreConnection::Incompatible(_) => {
                return Err(StartFailure::incompatible("verifying_runtime"));
            }
            CoreConnection::InvalidRuntime => {
                return Err(StartFailure::invalid_runtime("verifying_runtime"));
            }
            CoreConnection::Missing | CoreConnection::Stopped => {
                reporter.starting();
                started = Some(
                    dependencies
                        .service
                        .spawn(&executable)
                        .map_err(|error| StartFailure::start(error, "starting"))?,
                );
                wait_until_running(
                    runtime,
                    &executable,
                    dependencies,
                    started.as_mut().unwrap(),
                )
                .await?;
            }
            CoreConnection::Running(_) => unreachable!(),
        }
    }

    reporter.authorizing();
    let token =
        resolve_authorized_token(dependencies.service.as_ref(), runtime.client(), &executable)
            .await
            .map_err(|error| StartFailure::authorization(error, "authorizing"))?;

    reporter.verifying_runtime();
    match verify_authenticated_status(dependencies.service.as_ref(), runtime.client(), &token).await
    {
        Ok(()) => Ok(StartOutcome { already_running }),
        Err(failure) => {
            stop_started_core(started.as_deref_mut());
            Err(failure)
        }
    }
}

async fn run_development_start(
    runtime: &(impl CommandRuntime + Sync),
    dependencies: &StartDependencies,
    reporter: &mut StartProgressReporter<'_>,
    connection: CoreConnection,
) -> Result<StartOutcome, StartFailure> {
    match connection {
        CoreConnection::Running(_) => {
            let executable = runtime.executable().ok_or_else(|| {
                StartFailure::start(
                    CommandError::DevelopmentRuntimeManagedByIde,
                    "verifying_runtime",
                )
            })?;
            reporter.authorizing();
            let token = resolve_authorized_token(
                dependencies.service.as_ref(),
                runtime.client(),
                executable,
            )
            .await
            .map_err(|error| StartFailure::authorization(error, "authorizing"))?;
            reporter.verifying_runtime();
            match verify_authenticated_status(
                dependencies.service.as_ref(),
                runtime.client(),
                &token,
            )
            .await
            {
                Ok(()) => Ok(StartOutcome {
                    already_running: true,
                }),
                Err(failure) => Err(failure),
            }
        }
        CoreConnection::Incompatible(_) => Err(StartFailure::incompatible("verifying_runtime")),
        CoreConnection::InvalidRuntime => Err(StartFailure::invalid_runtime("verifying_runtime")),
        CoreConnection::Missing | CoreConnection::Stopped => Err(StartFailure::start(
            CommandError::DevelopmentRuntimeManagedByIde,
            "verifying_runtime",
        )),
    }
}

async fn verify_authenticated_status(
    service: &dyn StartService,
    client: &WokCoreClient,
    token: &SecretString,
) -> Result<(), StartFailure> {
    match service.authenticated_status(client, token).await {
        Ok(ServiceStatus {
            phase: ServicePhase::Running,
            ..
        }) => Ok(()),
        Ok(_) => Err(StartFailure::start(
            CommandError::StartFailed,
            "verifying_runtime",
        )),
        Err(ServiceError::Incompatible) => Err(StartFailure::incompatible("verifying_runtime")),
        Err(ServiceError::InvalidRuntime | ServiceError::InvalidResponse) => {
            Err(StartFailure::invalid_runtime("verifying_runtime"))
        }
        Err(ServiceError::Unauthorized | ServiceError::Forbidden) => Err(
            StartFailure::authorization(CommandError::AuthorizationRequired, "verifying_runtime"),
        ),
        Err(ServiceError::Missing | ServiceError::Stopped) => Err(StartFailure::start(
            CommandError::StartFailed,
            "verifying_runtime",
        )),
    }
}

async fn wait_until_running(
    runtime: &(impl CommandRuntime + Sync),
    executable: &Path,
    dependencies: &StartDependencies,
    child: &mut Box<dyn StartedCore>,
) -> Result<(), StartFailure> {
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        let connection = if runtime.establish_production_binding(executable) {
            dependencies.service.connection(runtime.client()).await
        } else {
            Ok(CoreConnection::Missing)
        };
        match connection {
            Ok(CoreConnection::Running(_)) => return Ok(()),
            Ok(CoreConnection::Incompatible(_)) => {
                stop_started_core(Some(child.as_mut()));
                return Err(StartFailure::incompatible("starting"));
            }
            Ok(CoreConnection::InvalidRuntime) => {
                stop_started_core(Some(child.as_mut()));
                return Err(StartFailure::invalid_runtime("starting"));
            }
            Ok(CoreConnection::Missing | CoreConnection::Stopped) => {}
            Err(error) => {
                stop_started_core(Some(child.as_mut()));
                return Err(StartFailure::start(error, "starting"));
            }
        }

        let child_exited = match child.try_wait() {
            Ok(exited) => exited,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(StartFailure::start(error, "starting"));
            }
        };
        if child_exited {
            return Err(StartFailure::start(CommandError::StartFailed, "starting"));
        }
        if Instant::now() >= deadline {
            stop_started_core(Some(child.as_mut()));
            return Err(StartFailure::start(CommandError::StartTimedOut, "starting"));
        }
        tokio::time::sleep_until(std::cmp::min(deadline, Instant::now() + RETRY_DELAY)).await;
    }
}

fn stop_started_core(child: Option<&mut (dyn StartedCore + '_)>) {
    let Some(child) = child else {
        return;
    };
    if matches!(child.try_wait(), Ok(true)) {
        return;
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn validate_options(options: StartOptions) -> Result<bool, CommandError> {
    match (options.json, options.progress_jsonl) {
        (false, false) => Ok(false),
        (true, true) => Ok(true),
        _ => Err(CommandError::Usage),
    }
}

#[derive(Clone, Copy)]
struct StartOutcome {
    already_running: bool,
}

struct StartFailure {
    command: CommandError,
    code: &'static str,
    phase: &'static str,
}

impl StartFailure {
    fn start(command: CommandError, phase: &'static str) -> Self {
        Self {
            command,
            code: "start_failed",
            phase,
        }
    }

    fn authorization(command: CommandError, phase: &'static str) -> Self {
        Self {
            command,
            code: "authorization_failed",
            phase,
        }
    }

    fn incompatible(phase: &'static str) -> Self {
        Self {
            command: CommandError::Incompatible,
            code: "incompatible_manifest",
            phase,
        }
    }

    fn invalid_runtime(phase: &'static str) -> Self {
        Self {
            command: CommandError::InvalidRuntime,
            code: "invalid_install_state",
            phase,
        }
    }

    fn install(error: WokCoreInstallError, phase: &'static str) -> Self {
        Self {
            command: CommandError::CoreControl,
            code: install_error_code(error),
            phase,
        }
    }
}

fn install_error_code(error: WokCoreInstallError) -> &'static str {
    match error {
        WokCoreInstallError::InvalidSource | WokCoreInstallError::DownloadFailed => {
            "download_failed"
        }
        WokCoreInstallError::InvalidInstallState => "invalid_install_state",
        WokCoreInstallError::InstallInProgress => "install_in_progress",
        WokCoreInstallError::InvalidManifest => "invalid_manifest",
        WokCoreInstallError::InvalidSignature => "invalid_signature",
        WokCoreInstallError::IncompatibleManifest => "incompatible_manifest",
        WokCoreInstallError::ArtifactSizeMismatch => "artifact_size_mismatch",
        WokCoreInstallError::ArtifactHashMismatch => "artifact_hash_mismatch",
        WokCoreInstallError::InvalidArchive => "invalid_archive",
        WokCoreInstallError::UnsafeInstallLocation => "unsafe_install_location",
        WokCoreInstallError::AtomicInstallFailed => "install_failed",
        WokCoreInstallError::InstallRecordFailed => "install_record_failed",
    }
}

async fn resolve_authorized_token(
    service: &dyn StartService,
    client: &WokCoreClient,
    executable: &Path,
) -> Result<SecretString, CommandError> {
    let token = service.authorize(client, executable).await?;
    match service.authorization_status(client, &token).await {
        Err(ServiceError::Unauthorized | ServiceError::Forbidden) => {
            service.reauthorize(client, executable).await
        }
        Ok(_) | Err(_) => Ok(token),
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

fn start_message(already_running: bool) -> &'static str {
    if already_running {
        "WokCore is already running."
    } else {
        "WokCore is running."
    }
}

#[cfg(test)]
mod tests;
