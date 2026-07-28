//! Platform-neutral operating system integration primitives.

pub mod service {
    pub mod manager;
}

pub mod system {
    pub mod locale;
    pub mod paths;
    pub mod wokcore;
}

pub use service::manager::{ServiceManager, ServiceStatus};
pub use system::locale::{SystemContext, detect_system_context};
pub use system::paths::AppPaths;
pub use system::wokcore::discover_wokcore_executable;

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("the operating system did not provide {name}")]
    MissingPlatformData { name: &'static str },
    #[error("service manager error: {message}")]
    Service { message: String },
    #[error("the WokCore install record is invalid")]
    InvalidWokCoreInstallRecord,
}
