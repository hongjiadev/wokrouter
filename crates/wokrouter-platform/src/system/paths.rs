use std::path::{Path, PathBuf};

use crate::PlatformError;

const APPLICATION_NAME: &str = "WokRouter";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppPaths {
    pub config_file: PathBuf,
    pub state_db: PathBuf,
    pub runtime_dir: PathBuf,
    pub log_dir: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self, PlatformError> {
        let config_dir = platform_config_dir()?;
        let state_dir = platform_state_dir()?;

        Ok(Self {
            config_file: config_dir.join("config.toml"),
            state_db: state_dir.join("state.sqlite3"),
            runtime_dir: platform_runtime_dir(&state_dir),
            log_dir: state_dir.join("logs"),
        })
    }
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
