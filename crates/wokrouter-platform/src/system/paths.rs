use std::path::{Path, PathBuf};

use crate::PlatformError;

const APPLICATION_NAME: &str = "WokRouter";
const WOKCORE_APPLICATION_NAME: &str = "WokCore";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppPaths {
    pub config_file: PathBuf,
    pub wokcore_install_record: PathBuf,
    pub wokcore_install_dir: PathBuf,
    pub runtime_dir: PathBuf,
    pub log_dir: PathBuf,
    pub wokcore_discovery_file: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self, PlatformError> {
        let config_dir = platform_config_dir()?;
        let state_dir = platform_state_dir()?;
        let wokcore_discovery_file = platform_wokcore_discovery_file(&state_dir)?;

        Ok(Self {
            config_file: config_dir.join("config.toml"),
            wokcore_install_record: config_dir.join("wokcore-install.json"),
            wokcore_install_dir: platform_wokcore_install_dir(&state_dir)?,
            runtime_dir: platform_runtime_dir(&state_dir),
            log_dir: state_dir.join("logs"),
            wokcore_discovery_file,
        })
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn platform_wokcore_install_dir(state_dir: &Path) -> Result<PathBuf, PlatformError> {
    let state_root = state_dir
        .parent()
        .ok_or(PlatformError::MissingPlatformData {
            name: "WokCore install directory",
        })?;
    Ok(state_root.join(WOKCORE_APPLICATION_NAME).join("bin"))
}

#[cfg(target_os = "linux")]
fn platform_wokcore_install_dir(_state_dir: &Path) -> Result<PathBuf, PlatformError> {
    Ok(xdg_directory("XDG_DATA_HOME", &[".local", "share"])?
        .join(WOKCORE_APPLICATION_NAME)
        .join("bin"))
}

#[cfg(any(windows, target_os = "macos"))]
fn platform_wokcore_discovery_file(state_dir: &Path) -> Result<PathBuf, PlatformError> {
    sibling_runtime_discovery(state_dir)
}

#[cfg(target_os = "linux")]
fn platform_wokcore_discovery_file(state_dir: &Path) -> Result<PathBuf, PlatformError> {
    if let Some(runtime_root) = environment_path("XDG_RUNTIME_DIR") {
        return Ok(runtime_root
            .join(WOKCORE_APPLICATION_NAME)
            .join("discovery.json"));
    }
    sibling_runtime_discovery(state_dir)
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn sibling_runtime_discovery(state_dir: &Path) -> Result<PathBuf, PlatformError> {
    let state_root = state_dir
        .parent()
        .ok_or(PlatformError::MissingPlatformData {
            name: "WokCore runtime directory",
        })?;
    let mut path = state_root.join(WOKCORE_APPLICATION_NAME);
    path.push("runtime");
    path.push("discovery.json");
    Ok(path)
}

#[cfg(windows)]
fn platform_config_dir() -> Result<PathBuf, PlatformError> {
    Ok(windows_data_dir("APPDATA", &["AppData", "Roaming"])?.join(APPLICATION_NAME))
}

#[cfg(windows)]
fn platform_state_dir() -> Result<PathBuf, PlatformError> {
    Ok(windows_data_dir("LOCALAPPDATA", &["AppData", "Local"])?.join(APPLICATION_NAME))
}

#[cfg(windows)]
fn platform_runtime_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("runtime")
}

#[cfg(target_os = "macos")]
fn platform_config_dir() -> Result<PathBuf, PlatformError> {
    Ok(home_dir()?
        .join("Library")
        .join("Application Support")
        .join(APPLICATION_NAME))
}

#[cfg(target_os = "macos")]
fn platform_state_dir() -> Result<PathBuf, PlatformError> {
    platform_config_dir()
}

#[cfg(target_os = "macos")]
fn platform_runtime_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("runtime")
}

#[cfg(target_os = "linux")]
fn platform_config_dir() -> Result<PathBuf, PlatformError> {
    Ok(xdg_directory("XDG_CONFIG_HOME", &[".config"])?.join(APPLICATION_NAME))
}

#[cfg(target_os = "linux")]
fn platform_state_dir() -> Result<PathBuf, PlatformError> {
    Ok(xdg_directory("XDG_STATE_HOME", &[".local", "state"])?.join(APPLICATION_NAME))
}

#[cfg(target_os = "linux")]
fn platform_runtime_dir(state_dir: &Path) -> PathBuf {
    environment_path("XDG_RUNTIME_DIR")
        .map(|path| path.join(APPLICATION_NAME))
        .unwrap_or_else(|| state_dir.join("runtime"))
}

#[cfg(windows)]
fn windows_data_dir(variable: &str, fallback: &[&str]) -> Result<PathBuf, PlatformError> {
    environment_path(variable)
        .or_else(|| {
            home_dir()
                .ok()
                .map(|home| append_components(home, fallback))
        })
        .ok_or(PlatformError::MissingPlatformData {
            name: "application data directory",
        })
}

#[cfg(target_os = "linux")]
fn xdg_directory(variable: &str, fallback: &[&str]) -> Result<PathBuf, PlatformError> {
    environment_path(variable)
        .or_else(|| {
            home_dir()
                .ok()
                .map(|home| append_components(home, fallback))
        })
        .ok_or(PlatformError::MissingPlatformData {
            name: "home directory",
        })
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn home_dir() -> Result<PathBuf, PlatformError> {
    environment_path("HOME")
        .or_else(|| environment_path("USERPROFILE"))
        .ok_or(PlatformError::MissingPlatformData {
            name: "home directory",
        })
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn environment_path(variable: &str) -> Option<PathBuf> {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

#[cfg(any(windows, target_os = "linux"))]
fn append_components(mut path: PathBuf, components: &[&str]) -> PathBuf {
    for component in components {
        path.push(component);
    }
    path
}
