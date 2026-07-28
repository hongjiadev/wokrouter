mod install;
mod manifest;

use std::{fmt, path::PathBuf, sync::Arc};

use semver::Version;
#[cfg(feature = "test-support")]
use url::Host;
use url::Url;

pub use install::install_missing_wokcore;

use self::manifest::validate_public_key;

const PRODUCTION_RELEASE_ORIGIN: &str =
    "https://github.com/hongjiadev/wokcore/releases/latest/download/";

#[derive(Clone)]
pub struct WokCoreInstallSource {
    pub(super) origin: Url,
    pub(super) public_key: Arc<str>,
    pub(super) production: bool,
}

impl WokCoreInstallSource {
    pub fn production(public_key: impl Into<Arc<str>>) -> Result<Self, WokCoreInstallError> {
        let public_key = public_key.into();
        validate_public_key(&public_key)?;
        Ok(Self {
            origin: Url::parse(PRODUCTION_RELEASE_ORIGIN)
                .map_err(|_| WokCoreInstallError::InvalidSource)?,
            public_key,
            production: true,
        })
    }

    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn loopback(
        origin: Url,
        public_key: impl Into<Arc<str>>,
    ) -> Result<Self, WokCoreInstallError> {
        let public_key = public_key.into();
        validate_public_key(&public_key)?;
        if origin.scheme() != "http"
            || !origin.username().is_empty()
            || origin.password().is_some()
            || origin.port().is_none_or(|port| port == 0)
            || origin.query().is_some()
            || origin.fragment().is_some()
            || !origin.path().ends_with('/')
            || !matches!(origin.host(), Some(Host::Ipv4(address)) if address.is_loopback())
        {
            return Err(WokCoreInstallError::InvalidSource);
        }
        Ok(Self {
            origin,
            public_key,
            production: false,
        })
    }

    pub fn origin(&self) -> &Url {
        &self.origin
    }
}

impl fmt::Debug for WokCoreInstallSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WokCoreInstallSource")
            .field("production", &self.production)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WokCoreInstallOutcome {
    Installed {
        version: Version,
        executable: PathBuf,
    },
    AlreadyInstalled {
        executable: PathBuf,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum WokCoreInstallError {
    #[error("the WokCore install source is invalid")]
    InvalidSource,
    #[error("the WokCore install state is invalid")]
    InvalidInstallState,
    #[error("another WokCore install is already in progress")]
    InstallInProgress,
    #[error("the WokCore release could not be downloaded")]
    DownloadFailed,
    #[error("the WokCore release manifest is malformed")]
    InvalidManifest,
    #[error("the WokCore release signature is invalid")]
    InvalidSignature,
    #[error("the WokCore release is incompatible with this platform")]
    IncompatibleManifest,
    #[error("the WokCore release artifact size does not match its manifest")]
    ArtifactSizeMismatch,
    #[error("the WokCore release artifact hash does not match its manifest")]
    ArtifactHashMismatch,
    #[error("the WokCore release archive is invalid")]
    InvalidArchive,
    #[error("the WokCore install directory is unsafe")]
    UnsafeInstallLocation,
    #[error("WokCore could not be installed atomically")]
    AtomicInstallFailed,
    #[error("the WokCore install record could not be saved")]
    InstallRecordFailed,
}
