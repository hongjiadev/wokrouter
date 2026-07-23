use std::path::Path;

use crate::PlatformError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceStatus {
    NotInstalled,
    Stopped,
    Running,
    Failed,
}

#[async_trait::async_trait]
pub trait ServiceManager: Send + Sync {
    async fn install(&self, executable: &Path) -> Result<(), PlatformError>;
    async fn start(&self) -> Result<(), PlatformError>;
    async fn stop(&self) -> Result<(), PlatformError>;
    async fn status(&self) -> Result<ServiceStatus, PlatformError>;
    async fn uninstall(&self) -> Result<(), PlatformError>;
}
