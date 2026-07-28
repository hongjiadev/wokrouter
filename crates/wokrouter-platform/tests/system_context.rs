use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use wokrouter_platform::{
    AppPaths, PlatformError, ServiceManager, ServiceStatus, detect_system_context,
};
use wokrouter_storage::UiConfig;

#[test]
fn explicit_locale_and_timezone_override_system_detection() {
    let ui = UiConfig {
        locale_override: Some("zh_CN".into()),
        timezone_override: Some("Asia/Shanghai".into()),
    };

    let context = detect_system_context(&ui);

    assert_eq!(context.locale, "zh-CN");
    assert_eq!(context.timezone, "Asia/Shanghai");
}

#[test]
fn invalid_overrides_are_not_returned_as_system_context() {
    let ui = UiConfig {
        locale_override: Some("not a locale".into()),
        timezone_override: Some("Not/A_Timezone".into()),
    };

    let context = detect_system_context(&ui);

    assert_ne!(context.locale, "not a locale");
    assert_ne!(context.timezone, "Not/A_Timezone");
}

#[test]
fn discovered_paths_are_not_derived_from_the_current_directory() {
    let paths = AppPaths::discover().expect("application paths should be discoverable");
    let current_directory = std::env::current_dir().expect("current directory should be available");

    assert!(!paths.config_file.starts_with(&current_directory));
    assert_eq!(
        paths.config_file.file_name().and_then(|name| name.to_str()),
        Some("config.toml")
    );
    assert_eq!(
        paths.state_db.file_name().and_then(|name| name.to_str()),
        Some("state.sqlite3")
    );
    assert_eq!(
        paths
            .wokcore_discovery_file
            .file_name()
            .and_then(|name| name.to_str()),
        Some("discovery.json")
    );
    assert!(
        paths
            .wokcore_discovery_file
            .components()
            .any(|component| component.as_os_str() == "WokCore")
    );
}

#[derive(Clone)]
struct FakeServiceManager {
    status: Arc<Mutex<ServiceStatus>>,
}

#[async_trait]
impl ServiceManager for FakeServiceManager {
    async fn install(&self, _executable: &Path) -> Result<(), PlatformError> {
        *self
            .status
            .lock()
            .expect("status mutex should not be poisoned") = ServiceStatus::Stopped;
        Ok(())
    }

    async fn start(&self) -> Result<(), PlatformError> {
        let mut status = self
            .status
            .lock()
            .expect("status mutex should not be poisoned");
        if *status == ServiceStatus::NotInstalled {
            return Err(PlatformError::Service {
                message: "service is not installed".into(),
            });
        }
        *status = ServiceStatus::Running;
        Ok(())
    }

    async fn stop(&self) -> Result<(), PlatformError> {
        *self
            .status
            .lock()
            .expect("status mutex should not be poisoned") = ServiceStatus::Stopped;
        Ok(())
    }

    async fn status(&self) -> Result<ServiceStatus, PlatformError> {
        Ok(*self
            .status
            .lock()
            .expect("status mutex should not be poisoned"))
    }

    async fn uninstall(&self) -> Result<(), PlatformError> {
        *self
            .status
            .lock()
            .expect("status mutex should not be poisoned") = ServiceStatus::NotInstalled;
        Ok(())
    }
}

#[tokio::test]
async fn service_manager_trait_supports_fake_lifecycle_and_failure() {
    let service = FakeServiceManager {
        status: Arc::new(Mutex::new(ServiceStatus::NotInstalled)),
    };

    assert!(service.start().await.is_err());
    service.install(Path::new("wokrouter.exe")).await.unwrap();
    service.start().await.unwrap();
    assert_eq!(service.status().await.unwrap(), ServiceStatus::Running);
    service.stop().await.unwrap();
    service.uninstall().await.unwrap();
    assert_eq!(service.status().await.unwrap(), ServiceStatus::NotInstalled);
}
