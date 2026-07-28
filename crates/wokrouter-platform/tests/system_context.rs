use wokrouter_platform::{AppPaths, detect_system_context};
use wokrouter_storage::UiConfig;

#[test]
fn default_context_is_detected_without_user_selection() {
    let context = detect_system_context(&UiConfig {
        locale_override: None,
        timezone_override: None,
    });

    assert!(!context.locale.is_empty());
    assert!(!context.timezone.is_empty());
}

#[test]
fn valid_internal_overrides_are_normalized() {
    let context = detect_system_context(&UiConfig {
        locale_override: Some("zh_CN".into()),
        timezone_override: Some("Asia/Shanghai".into()),
    });

    assert_eq!(context.locale, "zh-CN");
    assert_eq!(context.timezone, "Asia/Shanghai");
}

#[test]
fn invalid_overrides_are_not_returned_as_system_context() {
    let context = detect_system_context(&UiConfig {
        locale_override: Some("not a locale".into()),
        timezone_override: Some("Not/A_Timezone".into()),
    });

    assert_ne!(context.locale, "not a locale");
    assert_ne!(context.timezone, "Not/A_Timezone");
}

#[test]
fn discovered_paths_separate_wokrouter_state_from_wokcore_discovery() {
    let paths = AppPaths::discover().expect("application paths should be discoverable");
    let current_directory = std::env::current_dir().expect("current directory should be available");

    assert!(!paths.config_file.starts_with(&current_directory));
    assert_eq!(
        paths.config_file.file_name().and_then(|name| name.to_str()),
        Some("config.toml")
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
    assert_eq!(
        paths
            .wokcore_install_dir
            .file_name()
            .and_then(|name| name.to_str()),
        Some("bin")
    );
    assert!(
        paths
            .wokcore_install_dir
            .components()
            .any(|component| component.as_os_str() == "WokCore")
    );
    assert!(!paths.wokcore_install_dir.starts_with(&paths.runtime_dir));
}
