use std::{fs, process::Command};

use tempfile::tempdir;

#[test]
fn doctor_json_is_stable_and_does_not_mutate_fake_home() {
    let fixture = tempdir().unwrap();
    let home = fixture.path().join("home");
    let roaming = fixture.path().join("roaming");
    let local = fixture.path().join("local");
    let codex = home.join(".codex").join("config.toml");
    let claude = home.join(".claude").join("settings.json");
    fs::create_dir_all(codex.parent().unwrap()).unwrap();
    fs::create_dir_all(claude.parent().unwrap()).unwrap();
    fs::write(&codex, b"# native\nmodel = \"native-model\"\n").unwrap();
    fs::write(&claude, b"{\"theme\":\"dark\"}\n").unwrap();
    let before_codex = fs::read(&codex).unwrap();
    let before_claude = fs::read(&claude).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_wokrouter"))
        .args(["doctor", "--json"])
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env_remove("CODEX_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env("APPDATA", &roaming)
        .env("LOCALAPPDATA", &local)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema_version"], 1);
    assert_eq!(
        report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|check| check["id"] == "codex_config")
            .unwrap()["status"],
        "missing"
    );
    assert_eq!(fs::read(&codex).unwrap(), before_codex);
    assert_eq!(fs::read(&claude).unwrap(), before_claude);
    assert!(!local.join("WokRouter").join("integrations").exists());
    let rendered = String::from_utf8(output.stdout).unwrap();
    assert!(!rendered.contains("native-model"));
    assert!(!rendered.contains(fixture.path().to_string_lossy().as_ref()));
}

#[test]
fn doctor_honors_external_codex_and_claude_config_roots() {
    let fixture = tempdir().unwrap();
    let home = fixture.path().join("home");
    let codex_home = fixture.path().join("external-codex");
    let claude_home = fixture.path().join("external-claude");
    let roaming = fixture.path().join("roaming");
    let local = fixture.path().join("local");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&codex_home).unwrap();
    fs::create_dir_all(&claude_home).unwrap();
    fs::write(
        codex_home.join("config.toml"),
        b"model_provider = \"wokcore\"\n[model_providers.wokcore]\nbase_url = \"http://127.0.0.1:10101/v1/\"\n",
    )
    .unwrap();
    fs::write(
        claude_home.join("settings.json"),
        br#"{"apiKeyHelper":"wokrouter integration-token claude"}"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_wokrouter"))
        .args(["doctor", "--json"])
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("CODEX_HOME", &codex_home)
        .env("CLAUDE_CONFIG_DIR", &claude_home)
        .env("APPDATA", &roaming)
        .env("LOCALAPPDATA", &local)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    for id in ["codex_config", "claude_config"] {
        assert_eq!(
            report["checks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|check| check["id"] == id)
                .unwrap()["status"],
            "conflict"
        );
    }
    assert!(!local.join("WokRouter").join("integrations").exists());
}
