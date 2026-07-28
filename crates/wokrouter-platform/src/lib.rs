//! Platform-neutral WokRouter and WokCore discovery primitives.

mod client;
mod wokcore_install;

pub mod system {
    pub mod locale;
    pub mod paths;
    #[cfg(windows)]
    pub(crate) mod windows_security;
    pub mod wokcore;
}

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

#[cfg(all(windows, feature = "test-support"))]
#[doc(hidden)]
pub mod test_support {
    use std::{io, path::Path};

    use crate::system::windows_security::{
        PrivatePathKind, private_path_owned_by_current_user_and_system, secure_private_path,
    };

    pub fn secure_private_file(path: &Path) -> io::Result<()> {
        secure_private_path(path, PrivatePathKind::File)
    }

    pub fn is_private_file(path: &Path) -> bool {
        private_path_owned_by_current_user_and_system(path, PrivatePathKind::File)
    }

    pub fn is_private_directory(path: &Path) -> bool {
        private_path_owned_by_current_user_and_system(path, PrivatePathKind::Directory)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("the operating system did not provide {name}")]
    MissingPlatformData { name: &'static str },
    #[error("the WokCore install record is invalid")]
    InvalidWokCoreInstallRecord,
}
