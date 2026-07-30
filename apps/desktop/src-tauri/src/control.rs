use std::sync::Arc;

use tokio::sync::{Mutex, watch};
use wokrouter_cli::commands::{CommandError, CoreStatus, start, status::snapshot_selected, stop};

use crate::runtime::{DesktopRuntimeError, DesktopRuntimeState};

type StartResult = Result<(), DesktopControlError>;

#[derive(Default)]
struct StartGate {
    in_flight: Option<watch::Receiver<Option<StartResult>>>,
}

#[derive(Clone)]
pub(crate) struct DesktopControl {
    runtime: Arc<DesktopRuntimeState>,
    start_gate: Arc<Mutex<StartGate>>,
}

impl DesktopControl {
    pub(crate) fn new(runtime: Arc<DesktopRuntimeState>) -> Self {
        Self {
            runtime,
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
        start::execute(runtime)
            .await
            .map(|_| ())
            .map_err(map_start_error)
    }

    pub(crate) async fn stop(&self) -> Result<(), DesktopControlError> {
        let runtime = self.runtime.selected().await?;
        stop::execute(runtime)
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
