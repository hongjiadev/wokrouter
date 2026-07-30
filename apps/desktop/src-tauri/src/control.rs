use std::{future::Future, pin::Pin, sync::Arc};

use tokio::sync::{Mutex, watch};
use wokrouter_cli::commands::{CommandError, CoreStatus, start, status::snapshot_selected, stop};
use wokrouter_platform::SelectedWokCoreRuntime;

use crate::runtime::{DesktopRuntimeError, DesktopRuntimeState};

type StartResult = Result<(), DesktopControlError>;
pub(crate) type LifecycleFuture<'a> =
    Pin<Box<dyn Future<Output = Result<u8, CommandError>> + Send + 'a>>;

pub(crate) trait DesktopLifecycle: Send + Sync {
    fn start<'a>(&'a self, runtime: &'a SelectedWokCoreRuntime) -> LifecycleFuture<'a>;
    fn stop<'a>(&'a self, runtime: &'a SelectedWokCoreRuntime) -> LifecycleFuture<'a>;
}

struct SystemDesktopLifecycle;

impl DesktopLifecycle for SystemDesktopLifecycle {
    fn start<'a>(&'a self, runtime: &'a SelectedWokCoreRuntime) -> LifecycleFuture<'a> {
        Box::pin(start::execute(runtime))
    }

    fn stop<'a>(&'a self, runtime: &'a SelectedWokCoreRuntime) -> LifecycleFuture<'a> {
        Box::pin(stop::execute(runtime))
    }
}

#[derive(Default)]
struct StartGate {
    in_flight: Option<watch::Receiver<Option<StartResult>>>,
}

#[derive(Clone)]
pub(crate) struct DesktopControl {
    runtime: Arc<DesktopRuntimeState>,
    lifecycle: Arc<dyn DesktopLifecycle>,
    start_gate: Arc<Mutex<StartGate>>,
}

impl DesktopControl {
    pub(crate) fn new(runtime: Arc<DesktopRuntimeState>) -> Self {
        Self::new_with_lifecycle(runtime, Arc::new(SystemDesktopLifecycle))
    }

    pub(crate) fn new_with_lifecycle(
        runtime: Arc<DesktopRuntimeState>,
        lifecycle: Arc<dyn DesktopLifecycle>,
    ) -> Self {
        Self {
            runtime,
            lifecycle,
            start_gate: Arc::new(Mutex::new(StartGate::default())),
        }
    }

    pub(crate) async fn status(&self) -> Result<CoreStatus, DesktopControlError> {
        let runtime = self.runtime.selected().await?;
        snapshot_selected(runtime)
            .await
            .map(|(status, _)| status)
            .map_err(|_| DesktopControlError::StatusUnavailable)
    }

    pub(crate) async fn start(&self) -> StartResult {
        let mut receiver = {
            let mut gate = self.start_gate.lock().await;
            if let Some(receiver) = &gate.in_flight {
                receiver.clone()
            } else {
                let (sender, receiver) = watch::channel(None);
                gate.in_flight = Some(receiver.clone());
                let control = self.clone();
                tokio::spawn(async move {
                    let result = control.start_once().await;
                    control.start_gate.lock().await.in_flight = None;
                    sender.send_replace(Some(result));
                });
                receiver
            }
        };

        loop {
            if let Some(result) = { *receiver.borrow() } {
                return result;
            }
            receiver
                .changed()
                .await
                .map_err(|_| DesktopControlError::StartUnavailable)?;
        }
    }

    async fn start_once(&self) -> StartResult {
        let runtime = self.runtime.selected().await?;
        self.lifecycle
            .start(runtime)
            .await
            .map(|_| ())
            .map_err(map_start_error)
    }

    pub(crate) async fn stop(&self) -> Result<(), DesktopControlError> {
        let runtime = self.runtime.selected().await?;
        self.lifecycle
            .stop(runtime)
            .await
            .map(|_| ())
            .map_err(map_stop_error)
    }
}

fn map_start_error(error: CommandError) -> DesktopControlError {
    match error {
        CommandError::DevelopmentRuntimeManagedByIde => {
            DesktopControlError::DevelopmentRuntimeManagedByIde
        }
        _ => DesktopControlError::StartUnavailable,
    }
}

fn map_stop_error(error: CommandError) -> DesktopControlError {
    match error {
        CommandError::DevelopmentRuntimeManagedByIde => {
            DesktopControlError::DevelopmentRuntimeManagedByIde
        }
        _ => DesktopControlError::StopUnavailable,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum DesktopControlError {
    #[error("runtime_initialization_failed")]
    Initialization,
    #[error("status_unavailable")]
    StatusUnavailable,
    #[error("start_unavailable")]
    StartUnavailable,
    #[error("stop_unavailable")]
    StopUnavailable,
    #[error("development_runtime_managed_by_ide")]
    DevelopmentRuntimeManagedByIde,
}

impl From<DesktopRuntimeError> for DesktopControlError {
    fn from(_: DesktopRuntimeError) -> Self {
        Self::Initialization
    }
}
