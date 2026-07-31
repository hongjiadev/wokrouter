use std::{env, ffi::OsString, path::Path, process::Stdio, sync::Arc};

use tokio::{process::Command, sync::mpsc};
use uuid::Uuid;
use wokrouter_platform::{AppPaths, discover_recorded_wokcore_executable};

use super::{
    CoreOperationError, CoreOperationKind, CoreOperationPhase, CoreOperationSnapshot,
    CoreOperationState, OperationRequest, OperationRunner, RunnerError, SystemOperationRunner,
    final_operation_snapshot,
    journal::{LeaseAttempt, OperationJournal},
};

pub(super) const OPERATION_HELPER_FLAG: &str = "--wokrouter-core-operation-helper";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct OperationHelperInvocation {
    pub(super) operation_id: Uuid,
    pub(super) operation: CoreOperationKind,
}

pub(super) enum OperationHelperRequest {
    NotRequested,
    Invalid,
    Valid(OperationHelperInvocation),
}

pub(super) fn spawn_helper_process(
    executable: &Path,
    operation_id: Uuid,
    operation: CoreOperationKind,
) -> Result<tokio::process::Child, RunnerError> {
    let mut command = Command::new(executable);
    command
        .arg(OPERATION_HELPER_FLAG)
        .arg(operation_id.to_string())
        .arg(match operation {
            CoreOperationKind::Install => "install",
            CoreOperationKind::Update => "update",
        })
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(false);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    command.spawn().map_err(|_| RunnerError::Spawn)
}

pub(super) fn parse_operation_helper_request(
    arguments: impl IntoIterator<Item = OsString>,
) -> OperationHelperRequest {
    let mut arguments = arguments.into_iter();
    let Some(flag) = arguments.next() else {
        return OperationHelperRequest::NotRequested;
    };
    if flag != OPERATION_HELPER_FLAG {
        return OperationHelperRequest::NotRequested;
    }
    let (Some(operation_id), Some(operation), None) =
        (arguments.next(), arguments.next(), arguments.next())
    else {
        return OperationHelperRequest::Invalid;
    };
    let (Some(operation_id), Some(operation)) = (operation_id.to_str(), operation.to_str()) else {
        return OperationHelperRequest::Invalid;
    };
    let Ok(parsed_id) = Uuid::parse_str(operation_id) else {
        return OperationHelperRequest::Invalid;
    };
    if parsed_id.to_string() != operation_id {
        return OperationHelperRequest::Invalid;
    }
    let operation = match operation {
        "install" => CoreOperationKind::Install,
        "update" => CoreOperationKind::Update,
        _ => return OperationHelperRequest::Invalid,
    };
    OperationHelperRequest::Valid(OperationHelperInvocation {
        operation_id: parsed_id,
        operation,
    })
}

pub(super) fn run_operation_helper_if_requested() -> Option<u8> {
    let invocation = match parse_operation_helper_request(env::args_os().skip(1)) {
        OperationHelperRequest::NotRequested => return None,
        OperationHelperRequest::Invalid => return Some(1),
        OperationHelperRequest::Valid(invocation) => invocation,
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return Some(1),
    };
    Some(runtime.block_on(run_system_operation_helper(invocation)))
}

async fn run_system_operation_helper(invocation: OperationHelperInvocation) -> u8 {
    let paths = match AppPaths::discover() {
        Ok(paths) => paths,
        Err(_) => return 1,
    };
    let journal = match OperationJournal::open(&paths.runtime_dir) {
        Ok(journal) => Arc::new(journal),
        Err(_) => return 1,
    };
    let request = match invocation.operation {
        CoreOperationKind::Install => OperationRequest::Install,
        CoreOperationKind::Update => {
            let executable =
                match discover_recorded_wokcore_executable(&paths.wokcore_install_record) {
                    Ok(Some(executable)) => executable,
                    Ok(None) | Err(_) => {
                        return fail_helper_before_run(
                            &journal,
                            invocation,
                            "update_install_failed",
                        );
                    }
                };
            OperationRequest::Update { executable }
        }
    };
    run_operation_helper_with_request(
        journal,
        invocation.operation_id,
        request,
        Arc::new(SystemOperationRunner),
    )
    .await
}

fn fail_helper_before_run(
    journal: &OperationJournal,
    invocation: OperationHelperInvocation,
    error_code: &'static str,
) -> u8 {
    let lease = match journal.try_operation_lease() {
        Ok(LeaseAttempt::Acquired(lease)) => lease,
        Ok(LeaseAttempt::Busy) | Err(_) => return 1,
    };
    let result = journal.read().and_then(|snapshot| {
        let snapshot = snapshot.ok_or(CoreOperationError::InvalidProgress)?;
        if snapshot.operation_id != invocation.operation_id
            || snapshot.operation != invocation.operation
            || !helper_initial_snapshot_is_valid(&snapshot)
        {
            return Err(CoreOperationError::InvalidProgress);
        }
        journal.write(&CoreOperationSnapshot::failed(
            invocation.operation_id,
            snapshot.sequence.saturating_add(1),
            invocation.operation,
            error_code,
        ))
    });
    drop(lease);
    u8::from(result.is_err())
}

pub(super) async fn run_operation_helper_with_request(
    journal: Arc<OperationJournal>,
    operation_id: Uuid,
    request: OperationRequest,
    runner: Arc<dyn OperationRunner>,
) -> u8 {
    let operation = match &request {
        OperationRequest::Install => CoreOperationKind::Install,
        OperationRequest::Update { .. } => CoreOperationKind::Update,
    };
    let lease = match journal.try_operation_lease() {
        Ok(LeaseAttempt::Acquired(lease)) => lease,
        Ok(LeaseAttempt::Busy) | Err(_) => return 1,
    };
    let initial = match journal.read() {
        Ok(Some(snapshot))
            if snapshot.operation_id == operation_id
                && snapshot.operation == operation
                && helper_initial_snapshot_is_valid(&snapshot) =>
        {
            snapshot
        }
        _ => return 1,
    };

    let (sender, mut receiver) = mpsc::channel(32);
    let runner = tokio::spawn(runner.run(request, sender));
    let mut sequence = initial.sequence;
    let mut terminal = None;
    let mut progress_persisted = true;
    while let Some(event) = receiver.recv().await {
        if event.state == CoreOperationState::Running {
            sequence = match sequence.checked_add(1) {
                Some(sequence) => sequence,
                None => {
                    progress_persisted = false;
                    u64::MAX
                }
            };
            let snapshot = CoreOperationSnapshot::from_child(operation_id, sequence, event);
            if journal.write(&snapshot).is_err() {
                progress_persisted = false;
            }
        } else {
            terminal = Some(event);
        }
    }
    let result = match runner.await {
        Ok(result) => result,
        Err(_) => Err(RunnerError::Wait),
    };
    sequence = sequence.saturating_add(1);
    let snapshot = final_operation_snapshot(
        operation_id,
        sequence,
        operation,
        terminal,
        &result,
        progress_persisted,
    );
    let result = journal.write(&snapshot);
    drop(lease);
    u8::from(result.is_err())
}

fn helper_initial_snapshot_is_valid(snapshot: &CoreOperationSnapshot) -> bool {
    snapshot.sequence == 0
        && snapshot.state == CoreOperationState::Running
        && snapshot.phase == CoreOperationPhase::CheckingRelease
        && snapshot.current_version.is_none()
        && snapshot.target_version.is_none()
        && snapshot.bytes_completed.is_none()
        && snapshot.bytes_total.is_none()
        && snapshot.active_requests.is_none()
        && snapshot.error_code.is_none()
}
