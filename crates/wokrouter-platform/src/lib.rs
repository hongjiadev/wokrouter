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
pub use system::locale::{SystemContext, detect_system_context};
pub use system::paths::AppPaths;
pub use system::wokcore::discover_wokcore_executable;
pub use wokcore_install::{
    WokCoreInstallError, WokCoreInstallOutcome, WokCoreInstallSource, install_missing_wokcore,
};
pub use wokcore_runtime::{SelectedWokCoreRuntime, WokCoreRuntimeChannel, select_wokcore_runtime};

#[cfg(feature = "test-support")]
#[doc(hidden)]
pub mod test_support {
    use std::{io, path::Path};

    #[cfg(windows)]
    use crate::system::windows_security::{
        PrivatePathKind, private_path_owned_by_current_user_and_system, secure_private_path,
    };
    #[cfg(debug_assertions)]
    pub use crate::wokcore_runtime::test_support::RuntimeSelectorHarness;

    #[cfg(windows)]
    pub fn secure_private_file(path: &Path) -> io::Result<()> {
        secure_private_path(path, PrivatePathKind::File)
    }

    #[cfg(unix)]
    pub fn secure_private_file(path: &Path) -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
    }

    #[cfg(windows)]
    pub fn secure_private_directory(path: &Path) -> io::Result<()> {
        secure_private_path(path, PrivatePathKind::Directory)
    }

    #[cfg(unix)]
    pub fn secure_private_directory(path: &Path) -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
    }

    #[cfg(windows)]
    pub fn is_private_file(path: &Path) -> bool {
        private_path_owned_by_current_user_and_system(path, PrivatePathKind::File)
    }

    #[cfg(windows)]
    pub fn is_private_directory(path: &Path) -> bool {
        private_path_owned_by_current_user_and_system(path, PrivatePathKind::Directory)
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
