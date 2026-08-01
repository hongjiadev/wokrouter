use semver::Version;
use serde::Serialize;
use wokrouter_platform::{
    WokCoreInstallPhase, WokCoreInstallProgress, WokCoreInstallProgressObserver,
};

use super::StartCommandOutput;

#[derive(Serialize)]
struct CoreOperationProgress<'a> {
    schema_version: u8,
    sequence: u64,
    operation: &'static str,
    state: &'static str,
    phase: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_version: Option<&'a Version>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes_completed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<&'static str>,
}

pub(super) struct StartProgressReporter<'a> {
    output: &'a mut dyn StartCommandOutput,
    enabled: bool,
    sequence: u64,
    phase: &'static str,
    target_version: Option<Version>,
}

impl<'a> StartProgressReporter<'a> {
    pub(super) fn new(output: &'a mut dyn StartCommandOutput, progress_jsonl: bool) -> Self {
        Self {
            output,
            enabled: progress_jsonl,
            sequence: 0,
            phase: "checking_release",
            target_version: None,
        }
    }

    pub(super) fn starting(&mut self) {
        self.emit("running", "starting", None, None, None);
    }

    pub(super) fn authorizing(&mut self) {
        self.emit("running", "authorizing", None, None, None);
    }

    pub(super) fn verifying_runtime(&mut self) {
        self.emit("running", "verifying_runtime", None, None, None);
    }

    pub(super) fn completed(&mut self) {
        self.emit("succeeded", "completed", None, None, None);
    }

    pub(super) fn failed(&mut self, phase: &'static str, error_code: &'static str) {
        self.emit("failed", phase, None, None, Some(error_code));
    }

    pub(super) fn phase(&self) -> &'static str {
        self.phase
    }

    pub(super) fn stdout_code(&mut self, code: &'static str) {
        let _ = self.output.stdout(&format!("{{\"code\":\"{code}\"}}\n"));
    }

    pub(super) fn human_message(&mut self, message: &'static str) -> std::io::Result<()> {
        self.output.stdout(&format!("{message}\n"))
    }

    fn emit(
        &mut self,
        state: &'static str,
        phase: &'static str,
        bytes_completed: Option<u64>,
        bytes_total: Option<u64>,
        error_code: Option<&'static str>,
    ) {
        self.phase = phase;
        if !self.enabled {
            return;
        }
        let event = CoreOperationProgress {
            schema_version: 1,
            sequence: self.sequence,
            operation: "install",
            state,
            phase,
            target_version: self.target_version.as_ref(),
            bytes_completed,
            bytes_total,
            error_code,
        };
        let Ok(mut line) = serde_json::to_string(&event) else {
            self.enabled = false;
            return;
        };
        line.push('\n');
        if self.output.stderr(&line).is_err() {
            self.enabled = false;
            return;
        }
        self.sequence += 1;
    }
}

impl WokCoreInstallProgressObserver for StartProgressReporter<'_> {
    fn on_progress(&mut self, event: WokCoreInstallProgress) {
        if let Some(version) = event.target_version {
            self.target_version = Some(version);
        }
        let phase = match event.phase {
            WokCoreInstallPhase::CheckingRelease => "checking_release",
            WokCoreInstallPhase::Downloading => "downloading",
            WokCoreInstallPhase::Verifying => "verifying",
            WokCoreInstallPhase::Installing => "installing",
        };
        self.emit(
            "running",
            phase,
            event.bytes_completed,
            event.bytes_total,
            None,
        );
    }
}
