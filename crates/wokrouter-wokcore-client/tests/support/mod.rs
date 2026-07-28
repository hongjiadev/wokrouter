use std::{fs, path::Path};

use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

pub const INSTANCE_ID: &str = "01234567-89ab-4cde-8fab-0123456789ab";
pub const INSTALLATION_ID: &str =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

pub fn write_discovery(
    path: &Path,
    base_url: &str,
    instance_id: &str,
    api_major: u32,
    extra: Option<(&str, serde_json::Value)>,
) {
    let mut document = json!({
        "base_url": base_url,
        "pid": std::process::id(),
        "instance_id": instance_id,
        "wokcore_version": "0.1.0",
        "api_major": api_major
    });
    if let Some((name, value)) = extra {
        document.as_object_mut().unwrap().insert(name.into(), value);
    }
    fs::write(path, serde_json::to_vec(&document).unwrap()).unwrap();
    secure_file(path);
}

#[allow(dead_code)]
pub async fn mount_handshake(server: &MockServer, instance_id: &str) {
    let authority = server.uri().trim_start_matches("http://").to_owned();
    Mock::given(method("GET"))
        .and(path("/wokcore/v1/health"))
        .and(header("host", authority.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "ok",
            "instance_id": instance_id,
            "future_health_field": true
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/wokcore/v1/capabilities"))
        .and(header("host", authority.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "wokcore_version": "0.1.0",
            "management_api_major": 1,
            "minimum_management_api_major": 1,
            "maximum_management_api_major": 1,
            "provider_protocols": ["openai_responses", "anthropic_messages"],
            "capabilities": ["discovery.v1", "service.status"],
            "instance_id": instance_id,
            "installation_id": INSTALLATION_ID,
            "future_capability_field": {"enabled": true}
        })))
        .mount(server)
        .await;
}

#[cfg(unix)]
fn secure_file(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[cfg(not(unix))]
fn secure_file(_path: &Path) {}
