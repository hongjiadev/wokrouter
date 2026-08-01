//! Platform-neutral WokRouter and WokCore discovery primitives.

mod client;
pub mod system;
mod wokcore_install;
mod wokcore_runtime;

pub use client::{
    ClientIntegrationManager, ClientKind, ClientRoots, CopilotSetup, DoctorCheck, DoctorReport,
    DoctorSeverity, DoctorStatus, IntegrationDoctor, IntegrationError, IntegrationStatus,
    MutationError, MutationId, MutationJournal, MutationOperation, MutationStatus, OwnedMutation,
    PreparedMutation, RestoreResult,
};
pub use system::locale::{SystemContext, detect_system_context, detect_system_locale};
pub use system::paths::AppPaths;
#[doc(hidden)]
pub use system::private_paths::{
    is_private_directory, is_private_file, secure_private_directory, secure_private_file,
};
pub use system::wokcore::{discover_recorded_wokcore_executable, discover_wokcore_executable};
pub use wokcore_install::{
    WokCoreInstallError, WokCoreInstallOutcome, WokCoreInstallPhase, WokCoreInstallProgress,
    WokCoreInstallProgressObserver, WokCoreInstallSource, install_missing_wokcore,
    install_missing_wokcore_with_progress,
};
pub use wokcore_runtime::{SelectedWokCoreRuntime, WokCoreRuntimeChannel, select_wokcore_runtime};

#[doc(hidden)]
pub fn wokcore_install_lease_active(
    directory: &std::path::Path,
) -> Result<bool, WokCoreInstallError> {
    wokcore_install::install_lease_active(directory)
}

#[cfg(feature = "test-support")]
#[doc(hidden)]
pub mod test_support {
    use std::{io, path::Path};

    use crate::WokCoreInstallError;

    pub struct WokCoreInstallLease {
        _lease: crate::wokcore_install::InstallLease,
    }

    pub fn acquire_wokcore_install_lease(
        directory: &Path,
    ) -> Result<WokCoreInstallLease, WokCoreInstallError> {
        crate::wokcore_install::acquire_install_lease(directory)
            .map(|lease| WokCoreInstallLease { _lease: lease })
    }

    #[cfg(debug_assertions)]
    pub use crate::wokcore_runtime::test_support::RuntimeSelectorHarness;

    pub fn secure_private_file(path: &Path) -> io::Result<()> {
        crate::secure_private_file(path)
    }

    pub fn secure_private_directory(path: &Path) -> io::Result<()> {
        crate::secure_private_directory(path)
    }

    pub fn is_private_file(path: &Path) -> bool {
        crate::is_private_file(path)
    }

    pub fn is_private_directory(path: &Path) -> bool {
        crate::is_private_directory(path)
    }

    #[cfg(debug_assertions)]
    pub fn process_executable_matches(process_id: std::num::NonZeroU32, candidate: &Path) -> bool {
        crate::system::process_executable_matches(process_id, candidate)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("the operating system did not provide {name}")]
    MissingPlatformData { name: &'static str },
    #[error("the WokCore install record is invalid")]
    InvalidWokCoreInstallRecord,
    #[error("failed to initialize the WokCore client")]
    WokCoreClientInitialization,
}
