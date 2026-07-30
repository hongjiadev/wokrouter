use semver::Version;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WokCoreInstallPhase {
    CheckingRelease,
    Downloading,
    Verifying,
    Installing,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WokCoreInstallProgress {
    pub phase: WokCoreInstallPhase,
    pub target_version: Option<Version>,
    pub bytes_completed: Option<u64>,
    pub bytes_total: Option<u64>,
}

pub trait WokCoreInstallProgressObserver: Send {
    fn on_progress(&mut self, event: WokCoreInstallProgress);
}

pub(super) struct NoopInstallProgress;

impl WokCoreInstallProgressObserver for NoopInstallProgress {
    fn on_progress(&mut self, _event: WokCoreInstallProgress) {}
}
