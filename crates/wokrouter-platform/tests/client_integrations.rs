use std::fs;

use secrecy::{ExposeSecret, SecretString};
use serde_json::json;
use tempfile::tempdir;
use toml_edit::DocumentMut;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path, path_regex},
};
use wokrouter_platform::{
    ClientIntegrationManager, ClientKind, ClientRoots, DoctorStatus, IntegrationDoctor,
    IntegrationError, IntegrationStatus, RestoreResult,
};
use wokrouter_wokcore_client::WokCoreClient;

const INSTANCE_ID: &str = "01234567-89ab-4cde-8fab-0123456789ab";
const RESTARTED_INSTANCE_ID: &str = "11234567-89ab-4cde-8fab-0123456789ab";
const MANAGEMENT_TOKEN: &str = "wok_proxy_v1_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const CLIENT_TOKEN: &str = "wok_proxy_v1_BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

#[cfg(all(windows, feature = "test-support"))]
#[test]
fn windows_discovery_fixture_is_private_for_current_user() {
    let fixture = tempdir().unwrap();
    write_discovery_url(&fixture, "http://127.0.0.1:8765", INSTANCE_ID);

    assert!(wokrouter_platform::test_support::is_private_file(
        &fixture.path().join("discovery.json")
    ));
}

#[tokio::test]
async fn codex_injection_preserves_native_text_and_uses_command_backed_auth() {
    let fixture = tempdir().unwrap();
    let roots = client_roots(&fixture);
    fs::create_dir_all(roots.codex_config.parent().unwrap()).unwrap();
    let native = "# native comment\r\nmodel = \"native-model\"\r\n";
    fs::write(&roots.codex_config, native).unwrap();
    let (server, core) = verified_core(&fixture).await;
    mount_issue(&server, ClientKind::Codex).await;
    let manager = ClientIntegrationManager::new(
        roots.clone(),
        fixture.path().join("state"),
        fixture.path().join("bin").join("wokrouter"),
    )
    .unwrap();

    let first = manager
        .inject(
            ClientKind::Codex,
            &core,
            &SecretString::from(MANAGEMENT_TOKEN.to_owned()),
        )
        .await
        .unwrap();
    let first_bytes = fs::read(&roots.codex_config).unwrap();
    let second = manager
        .inject(
            ClientKind::Codex,
            &core,
            &SecretString::from(MANAGEMENT_TOKEN.to_owned()),
        )
        .await
        .unwrap();

    assert!(matches!(first, IntegrationStatus::Injected { .. }));
    assert_eq!(first, second);
    assert_eq!(fs::read(&roots.codex_config).unwrap(), first_bytes);
    let injected = String::from_utf8(first_bytes).unwrap();
    assert!(injected.contains("# native comment\r\n"));
    assert!(injected.contains("model_provider = \"wokcore\""));
    assert!(injected.contains("[model_providers.wokcore.auth]"));
    assert!(injected.contains("integration-token"));
    assert!(!injected.contains("wokrouter_client_integration"));
    assert!(!injected.contains(CLIENT_TOKEN));
    let parsed = injected.parse::<DocumentMut>().unwrap();
    assert!(
        parsed
            .as_table()
            .iter()
            .all(|(key, _)| matches!(key, "model" | "model_provider" | "model_providers"))
    );
    assert_eq!(
        manager
            .read_token(ClientKind::Codex)
            .unwrap()
            .expose_secret(),
        CLIENT_TOKEN
    );
}

#[tokio::test]
async fn claude_injection_is_capability_gated_and_restores_exactly() {
    let fixture = tempdir().unwrap();
    let roots = client_roots(&fixture);
    fs::create_dir_all(roots.claude_settings.parent().unwrap()).unwrap();
    let native = br#"{
  "permissions": {"allow": ["Read"]},
  "theme": "dark"
}
"#;
    fs::write(&roots.claude_settings, native).unwrap();
    let (server, core) = verified_core(&fixture).await;
    mount_issue(&server, ClientKind::Claude).await;
    mount_revoke(&server, ClientKind::Claude).await;
    let manager = ClientIntegrationManager::new(
        roots.clone(),
        fixture.path().join("state"),
        fixture.path().join("bin").join("wokrouter"),
    )
    .unwrap();

    manager
        .inject(
            ClientKind::Claude,
            &core,
            &SecretString::from(MANAGEMENT_TOKEN.to_owned()),
        )
        .await
        .unwrap();
    let injected: serde_json::Value =
        serde_json::from_slice(&fs::read(&roots.claude_settings).unwrap()).unwrap();
    assert_eq!(
        injected["env"]["ANTHROPIC_BASE_URL"],
        format!("{}/", server.uri())
    );
    assert!(
        injected["apiKeyHelper"]
            .as_str()
            .unwrap()
            .contains("integration-token")
    );
    assert!(!injected.to_string().contains(CLIENT_TOKEN));
    assert_eq!(
        manager
            .read_token(ClientKind::Claude)
            .unwrap()
            .expose_secret(),
        CLIENT_TOKEN
    );

    let restored = manager
        .restore(
            ClientKind::Claude,
            &core,
            &SecretString::from(MANAGEMENT_TOKEN.to_owned()),
        )
        .await
        .unwrap();
    assert_eq!(restored, RestoreResult::Restored);
    assert_eq!(fs::read(&roots.claude_settings).unwrap(), native);
    assert!(manager.read_token(ClientKind::Claude).is_err());
}

#[tokio::test]
async fn copilot_setup_is_structured_and_never_mutates_opaque_app_data() {
    let fixture = tempdir().unwrap();
    let roots = client_roots(&fixture);
    fs::create_dir_all(&roots.copilot_data).unwrap();
    let opaque = roots.copilot_data.join("opaque.db");
    fs::write(&opaque, b"opaque-user-data").unwrap();
    let (server, core) = verified_core(&fixture).await;
    mount_issue(&server, ClientKind::Copilot).await;
    mount_revoke(&server, ClientKind::Copilot).await;
    let manager = ClientIntegrationManager::new(
        roots,
        fixture.path().join("state"),
        fixture.path().join("bin").join("wokrouter"),
    )
    .unwrap();

    let setup = manager
        .copilot_setup(&core, &SecretString::from(MANAGEMENT_TOKEN.to_owned()))
        .await
        .unwrap();

    assert_eq!(setup.base_url, format!("{}/v1/", server.uri()));
    assert_eq!(setup.provider_type, "openai");
    assert_eq!(setup.api_format, "chat_completions");
    assert_eq!(setup.api_key_command.last().unwrap(), "copilot");
    assert_eq!(fs::read(&opaque).unwrap(), b"opaque-user-data");
    assert!(!format!("{setup:?}").contains(CLIENT_TOKEN));
    assert_eq!(
        manager
            .restore(
                ClientKind::Copilot,
                &core,
                &SecretString::from(MANAGEMENT_TOKEN.to_owned()),
            )
            .await
            .unwrap(),
        RestoreResult::ManualActionRequired
    );
    assert_eq!(fs::read(&opaque).unwrap(), b"opaque-user-data");
    assert!(manager.read_token(ClientKind::Copilot).is_err());
}

#[tokio::test]
async fn injection_refuses_remote_issue_when_the_intent_wal_cannot_be_written() {
    let fixture = tempdir().unwrap();
    let roots = client_roots(&fixture);
    fs::create_dir_all(roots.codex_config.parent().unwrap()).unwrap();
    let native = b"# native\nmodel = \"native-model\"\n";
    fs::write(&roots.codex_config, native).unwrap();
    let (_server, core) = verified_core(&fixture).await;
    let state = fixture.path().join("state");
    let manager = ClientIntegrationManager::new(
        roots.clone(),
        state.clone(),
        fixture.path().join("bin").join("wokrouter"),
    )
    .unwrap();
    let blocked_registry = state.join("registry").join("codex.json");
    fs::create_dir(&blocked_registry).unwrap();

    assert!(
        manager
            .inject(
                ClientKind::Codex,
                &core,
                &SecretString::from(MANAGEMENT_TOKEN.to_owned()),
            )
            .await
            .is_err()
    );

    assert_eq!(fs::read(&roots.codex_config).unwrap(), native);
    assert!(manager.read_token(ClientKind::Codex).is_err());
}

#[tokio::test]
async fn pending_wal_recovers_a_remote_token_issued_before_local_token_commit() {
    let fixture = tempdir().unwrap();
    let roots = client_roots(&fixture);
    fs::create_dir_all(roots.codex_config.parent().unwrap()).unwrap();
    let native = b"model = \"native-model\"\n";
    fs::write(&roots.codex_config, native).unwrap();
    let (server, core) = verified_core(&fixture).await;
    mount_issue(&server, ClientKind::Codex).await;
    mount_revoke(&server, ClientKind::Codex).await;
    let state = fixture.path().join("state");
    let manager = ClientIntegrationManager::new(
        roots.clone(),
        state.clone(),
        fixture.path().join("bin").join("wokrouter"),
    )
    .unwrap();
    let management = SecretString::from(MANAGEMENT_TOKEN.to_owned());
    manager
        .inject(ClientKind::Codex, &core, &management)
        .await
        .unwrap();
    let registry = state.join("registry").join("codex.json");
    let mut record: serde_json::Value =
        serde_json::from_slice(&fs::read(&registry).unwrap()).unwrap();
    record["phase"] = serde_json::Value::String("preparing".to_owned());
    record["mutation_ids"] = json!([]);
    record["config_hash"] = serde_json::Value::Null;
    fs::write(&registry, serde_json::to_vec(&record).unwrap()).unwrap();
    secure_test_file(&registry);
    fs::remove_file(state.join("tokens").join("codex.token")).unwrap();
    fs::write(&roots.codex_config, native).unwrap();

    assert_eq!(
        manager
            .restore(ClientKind::Codex, &core, &management)
            .await
            .unwrap(),
        RestoreResult::AlreadyRestored
    );
    assert!(!registry.exists());
    assert_eq!(fs::read(&roots.codex_config).unwrap(), native);
}

#[tokio::test]
async fn pending_wal_recovers_a_committed_config_before_registry_activation() {
    let fixture = tempdir().unwrap();
    let roots = client_roots(&fixture);
    fs::create_dir_all(roots.codex_config.parent().unwrap()).unwrap();
    let native = b"model = \"native-model\"\n";
    fs::write(&roots.codex_config, native).unwrap();
    let (server, core) = verified_core(&fixture).await;
    mount_issue(&server, ClientKind::Codex).await;
    mount_revoke(&server, ClientKind::Codex).await;
    let state = fixture.path().join("state");
    let manager = ClientIntegrationManager::new(
        roots.clone(),
        state.clone(),
        fixture.path().join("bin").join("wokrouter"),
    )
    .unwrap();
    let management = SecretString::from(MANAGEMENT_TOKEN.to_owned());
    manager
        .inject(ClientKind::Codex, &core, &management)
        .await
        .unwrap();
    let registry = state.join("registry").join("codex.json");
    let mut record: serde_json::Value =
        serde_json::from_slice(&fs::read(&registry).unwrap()).unwrap();
    record["phase"] = serde_json::Value::String("preparing".to_owned());
    fs::write(&registry, serde_json::to_vec(&record).unwrap()).unwrap();
    secure_test_file(&registry);

    assert_eq!(
        manager
            .restore(ClientKind::Codex, &core, &management)
            .await
            .unwrap(),
        RestoreResult::AlreadyRestored
    );
    assert!(!registry.exists());
    assert_eq!(fs::read(&roots.codex_config).unwrap(), native);
    assert!(manager.read_token(ClientKind::Codex).is_err());
}

#[tokio::test]
async fn invalid_registry_identifiers_fail_closed() {
    let fixture = tempdir().unwrap();
    let roots = client_roots(&fixture);
    fs::create_dir_all(roots.codex_config.parent().unwrap()).unwrap();
    fs::write(&roots.codex_config, b"model = \"native-model\"\n").unwrap();
    let (server, core) = verified_core(&fixture).await;
    mount_issue(&server, ClientKind::Codex).await;
    let state = fixture.path().join("state");
    let manager = ClientIntegrationManager::new(
        roots,
        state.clone(),
        fixture.path().join("bin").join("wokrouter"),
    )
    .unwrap();
    manager
        .inject(
            ClientKind::Codex,
            &core,
            &SecretString::from(MANAGEMENT_TOKEN.to_owned()),
        )
        .await
        .unwrap();
    let registry = state.join("registry").join("codex.json");
    let mut record: serde_json::Value =
        serde_json::from_slice(&fs::read(&registry).unwrap()).unwrap();
    record["token_id"] = serde_json::Value::String("../outside".to_owned());
    fs::write(&registry, serde_json::to_vec(&record).unwrap()).unwrap();

    assert!(manager.status(ClientKind::Codex).is_err());
}

#[tokio::test]
async fn repair_rotates_a_missing_private_token_without_rewriting_the_config() {
    let fixture = tempdir().unwrap();
    let roots = client_roots(&fixture);
    fs::create_dir_all(roots.codex_config.parent().unwrap()).unwrap();
    fs::write(&roots.codex_config, b"model = \"native-model\"\n").unwrap();
    let (server, core) = verified_core(&fixture).await;
    mount_issue_times(&server, ClientKind::Codex, 2).await;
    mount_revoke(&server, ClientKind::Codex).await;
    let state = fixture.path().join("state");
    let manager = ClientIntegrationManager::new(
        roots.clone(),
        state.clone(),
        fixture.path().join("bin").join("wokrouter"),
    )
    .unwrap();
    let management_token = SecretString::from(MANAGEMENT_TOKEN.to_owned());
    manager
        .inject(ClientKind::Codex, &core, &management_token)
        .await
        .unwrap();
    let injected = fs::read(&roots.codex_config).unwrap();
    fs::remove_file(state.join("tokens").join("codex.token")).unwrap();

    let repaired = manager
        .repair(ClientKind::Codex, &core, &management_token)
        .await
        .unwrap();

    assert!(matches!(repaired, IntegrationStatus::Injected { .. }));
    assert_eq!(fs::read(&roots.codex_config).unwrap(), injected);
    assert_eq!(
        manager
            .read_token(ClientKind::Codex)
            .unwrap()
            .expose_secret(),
        CLIENT_TOKEN
    );
}

#[tokio::test]
async fn status_and_doctor_detect_owned_config_drift_without_rewriting_it() {
    let fixture = tempdir().unwrap();
    let roots = client_roots(&fixture);
    fs::create_dir_all(roots.codex_config.parent().unwrap()).unwrap();
    fs::write(&roots.codex_config, b"model = \"native-model\"\n").unwrap();
    let (server, core) = verified_core(&fixture).await;
    mount_issue(&server, ClientKind::Codex).await;
    let manager = ClientIntegrationManager::new(
        roots.clone(),
        fixture.path().join("state"),
        fixture.path().join("bin").join("wokrouter"),
    )
    .unwrap();
    manager
        .inject(
            ClientKind::Codex,
            &core,
            &SecretString::from(MANAGEMENT_TOKEN.to_owned()),
        )
        .await
        .unwrap();
    let edited = fs::read_to_string(&roots.codex_config)
        .unwrap()
        .replace(&server.uri(), "http://127.0.0.1:9");
    fs::write(&roots.codex_config, edited.as_bytes()).unwrap();

    assert_eq!(
        manager.status(ClientKind::Codex).unwrap(),
        IntegrationStatus::Drifted
    );
    let report = IntegrationDoctor::inspect(&manager).unwrap();
    assert_eq!(
        report
            .checks
            .iter()
            .find(|check| check.id == "codex_config")
            .unwrap()
            .status,
        DoctorStatus::Drifted
    );
    assert_eq!(fs::read(&roots.codex_config).unwrap(), edited.as_bytes());
}

#[tokio::test]
async fn doctor_reports_a_server_revoked_token_separately_from_local_state() {
    let fixture = tempdir().unwrap();
    let roots = client_roots(&fixture);
    fs::create_dir_all(roots.codex_config.parent().unwrap()).unwrap();
    fs::write(&roots.codex_config, b"model = \"native-model\"\n").unwrap();
    let (server, core) = verified_core(&fixture).await;
    mount_issue_times(&server, ClientKind::Codex, 2).await;
    let manager = ClientIntegrationManager::new(
        roots,
        fixture.path().join("state"),
        fixture.path().join("bin").join("wokrouter"),
    )
    .unwrap();
    let management = SecretString::from(MANAGEMENT_TOKEN.to_owned());
    manager
        .inject(ClientKind::Codex, &core, &management)
        .await
        .unwrap();
    Mock::given(method("GET"))
        .and(path_regex(
            r"^/wokcore/v1/clients/wokrouter\.codex/tokens/[0-9a-f-]+$",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"active": false})))
        .with_priority(1)
        .mount(&server)
        .await;

    let report = IntegrationDoctor::inspect_with_runtime(&manager, &core, Some(&management))
        .await
        .unwrap();

    assert_eq!(
        doctor_check(&report, "codex_runtime").status,
        DoctorStatus::Healthy
    );
    assert_eq!(
        doctor_check(&report, "codex_token_remote").status,
        DoctorStatus::Missing
    );
    assert_eq!(
        doctor_check(&report, "codex_token").status,
        DoctorStatus::Missing
    );
    assert_eq!(
        doctor_check(&report, "codex_token_remote")
            .remediation
            .as_deref(),
        Some("doctor --repair codex_token")
    );
    assert!(matches!(
        manager
            .repair(ClientKind::Codex, &core, &management)
            .await
            .unwrap(),
        IntegrationStatus::Injected { .. }
    ));
}

#[tokio::test]
async fn doctor_reports_stopped_identity_mismatch_and_removed_capabilities() {
    let fixture = tempdir().unwrap();
    let roots = client_roots(&fixture);
    fs::create_dir_all(roots.codex_config.parent().unwrap()).unwrap();
    fs::write(&roots.codex_config, b"model = \"native-model\"\n").unwrap();
    let (server, core) = verified_core(&fixture).await;
    mount_issue(&server, ClientKind::Codex).await;
    let state = fixture.path().join("state");
    let manager = ClientIntegrationManager::new(
        roots,
        state.clone(),
        fixture.path().join("bin").join("wokrouter"),
    )
    .unwrap();
    let management = SecretString::from(MANAGEMENT_TOKEN.to_owned());
    manager
        .inject(ClientKind::Codex, &core, &management)
        .await
        .unwrap();

    write_discovery_url(&fixture, "http://127.0.0.1:9", INSTANCE_ID);
    let stopped = IntegrationDoctor::inspect_with_runtime(&manager, &core, Some(&management))
        .await
        .unwrap();
    assert_eq!(
        doctor_check(&stopped, "codex_runtime").status,
        DoctorStatus::Missing
    );

    write_discovery(&fixture, &server);
    let registry = state.join("registry").join("codex.json");
    let mut record: serde_json::Value =
        serde_json::from_slice(&fs::read(&registry).unwrap()).unwrap();
    record["runtime"]["installation_id"] = serde_json::Value::String("b".repeat(64));
    fs::write(&registry, serde_json::to_vec(&record).unwrap()).unwrap();
    secure_test_file(&registry);
    let mismatch = IntegrationDoctor::inspect_with_runtime(&manager, &core, Some(&management))
        .await
        .unwrap();
    assert_eq!(
        doctor_check(&mismatch, "codex_runtime").status,
        DoctorStatus::Conflict
    );

    record["runtime"]["installation_id"] = serde_json::Value::String("a".repeat(64));
    fs::write(&registry, serde_json::to_vec(&record).unwrap()).unwrap();
    secure_test_file(&registry);
    let unsupported_server = MockServer::start().await;
    mount_runtime(&unsupported_server, &["client_token.inspect"]).await;
    write_discovery(&fixture, &unsupported_server);
    let unsupported = IntegrationDoctor::inspect_with_runtime(&manager, &core, Some(&management))
        .await
        .unwrap();
    assert_eq!(
        doctor_check(&unsupported, "codex_runtime").status,
        DoctorStatus::Unsupported
    );
}

#[tokio::test]
async fn doctor_marks_a_same_installation_runtime_change_as_repairable_config_drift() {
    let fixture = tempdir().unwrap();
    let roots = client_roots(&fixture);
    fs::create_dir_all(roots.codex_config.parent().unwrap()).unwrap();
    fs::write(&roots.codex_config, b"model = \"native-model\"\n").unwrap();
    let (first_server, core) = verified_core(&fixture).await;
    mount_issue(&first_server, ClientKind::Codex).await;
    let manager = ClientIntegrationManager::new(
        roots.clone(),
        fixture.path().join("state"),
        fixture.path().join("bin").join("wokrouter"),
    )
    .unwrap();
    let management = SecretString::from(MANAGEMENT_TOKEN.to_owned());
    manager
        .inject(ClientKind::Codex, &core, &management)
        .await
        .unwrap();

    let restarted = MockServer::start().await;
    mount_runtime_with_identity(
        &restarted,
        &[
            "client_token.issue",
            "client_token.inspect",
            "client_token.revoke",
        ],
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        RESTARTED_INSTANCE_ID,
    )
    .await;
    Mock::given(method("GET"))
        .and(path_regex(
            r"^/wokcore/v1/clients/wokrouter\.codex/tokens/[0-9a-f-]+$",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"active": false})))
        .with_priority(1)
        .mount(&restarted)
        .await;
    mount_issue(&restarted, ClientKind::Codex).await;
    write_discovery_url(&fixture, &restarted.uri(), RESTARTED_INSTANCE_ID);

    let drifted = IntegrationDoctor::inspect_with_runtime(&manager, &core, Some(&management))
        .await
        .unwrap();
    assert_eq!(
        doctor_check(&drifted, "codex_config").status,
        DoctorStatus::Drifted
    );
    assert_eq!(
        doctor_check(&drifted, "codex_config")
            .remediation
            .as_deref(),
        Some("doctor --repair codex_config")
    );
    assert_eq!(
        doctor_check(&drifted, "codex_runtime").status,
        DoctorStatus::Drifted
    );

    manager
        .repair(ClientKind::Codex, &core, &management)
        .await
        .unwrap();
    assert!(
        fs::read_to_string(&roots.codex_config)
            .unwrap()
            .contains(&format!("{}/v1/", restarted.uri()))
    );
    let repaired = IntegrationDoctor::inspect_with_runtime(&manager, &core, Some(&management))
        .await
        .unwrap();
    assert_eq!(
        doctor_check(&repaired, "codex_config").status,
        DoctorStatus::Healthy
    );
    assert_eq!(
        doctor_check(&repaired, "codex_runtime").status,
        DoctorStatus::Healthy
    );
}

#[tokio::test]
async fn copilot_runtime_change_reports_and_repairs_the_manual_byok_setup() {
    let fixture = tempdir().unwrap();
    let roots = client_roots(&fixture);
    fs::create_dir_all(&roots.copilot_data).unwrap();
    let (first_server, core) = verified_core(&fixture).await;
    mount_issue(&first_server, ClientKind::Copilot).await;
    let manager = ClientIntegrationManager::new(
        roots,
        fixture.path().join("state"),
        fixture.path().join("bin").join("wokrouter"),
    )
    .unwrap();
    let management = SecretString::from(MANAGEMENT_TOKEN.to_owned());
    manager.copilot_setup(&core, &management).await.unwrap();

    let restarted = MockServer::start().await;
    mount_runtime_with_identity(
        &restarted,
        &[
            "client_token.issue",
            "client_token.inspect",
            "client_token.revoke",
        ],
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        RESTARTED_INSTANCE_ID,
    )
    .await;
    write_discovery_url(&fixture, &restarted.uri(), RESTARTED_INSTANCE_ID);

    let drifted = IntegrationDoctor::inspect_with_runtime(&manager, &core, Some(&management))
        .await
        .unwrap();
    assert_eq!(
        doctor_check(&drifted, "copilot_config").status,
        DoctorStatus::Drifted
    );
    assert_eq!(
        doctor_check(&drifted, "copilot_config")
            .remediation
            .as_deref(),
        Some("doctor --repair copilot_config")
    );

    manager
        .repair(ClientKind::Copilot, &core, &management)
        .await
        .unwrap();
    let setup = manager.copilot_setup(&core, &management).await.unwrap();
    assert_eq!(setup.base_url, format!("{}/v1/", restarted.uri()));
    let repaired = IntegrationDoctor::inspect_with_runtime(&manager, &core, Some(&management))
        .await
        .unwrap();
    assert_eq!(
        doctor_check(&repaired, "copilot_runtime").status,
        DoctorStatus::Healthy
    );
}

#[tokio::test]
async fn restore_never_sends_an_old_token_id_to_a_different_installation() {
    let fixture = tempdir().unwrap();
    let roots = client_roots(&fixture);
    fs::create_dir_all(roots.codex_config.parent().unwrap()).unwrap();
    fs::write(&roots.codex_config, b"model = \"native-model\"\n").unwrap();
    let (first_server, core) = verified_core(&fixture).await;
    mount_issue(&first_server, ClientKind::Codex).await;
    let manager = ClientIntegrationManager::new(
        roots.clone(),
        fixture.path().join("state"),
        fixture.path().join("bin").join("wokrouter"),
    )
    .unwrap();
    let management = SecretString::from(MANAGEMENT_TOKEN.to_owned());
    manager
        .inject(ClientKind::Codex, &core, &management)
        .await
        .unwrap();
    let injected = fs::read(&roots.codex_config).unwrap();

    let other_installation = MockServer::start().await;
    mount_runtime_with_installation(
        &other_installation,
        &[
            "client_token.issue",
            "client_token.inspect",
            "client_token.revoke",
        ],
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
    .await;
    write_discovery(&fixture, &other_installation);

    assert_eq!(
        manager
            .restore(ClientKind::Codex, &core, &management)
            .await
            .unwrap_err(),
        IntegrationError::RuntimeChanged
    );
    assert_eq!(fs::read(&roots.codex_config).unwrap(), injected);
    assert!(manager.read_token(ClientKind::Codex).is_ok());
}

#[tokio::test]
async fn injection_requires_both_issue_and_revoke_capabilities() {
    let fixture = tempdir().unwrap();
    let roots = client_roots(&fixture);
    fs::create_dir_all(roots.codex_config.parent().unwrap()).unwrap();
    fs::write(&roots.codex_config, b"model = \"native-model\"\n").unwrap();
    let (_server, core) = verified_core_with_capabilities(&fixture, &["client_token.issue"]).await;
    let manager = ClientIntegrationManager::new(
        roots,
        fixture.path().join("state"),
        fixture.path().join("bin").join("wokrouter"),
    )
    .unwrap();

    assert_eq!(
        manager
            .inject(
                ClientKind::Codex,
                &core,
                &SecretString::from(MANAGEMENT_TOKEN.to_owned()),
            )
            .await
            .unwrap_err(),
        IntegrationError::Unsupported
    );
}

#[tokio::test]
async fn concurrent_injections_share_one_serializable_client_transaction() {
    let fixture = tempdir().unwrap();
    let roots = client_roots(&fixture);
    fs::create_dir_all(roots.codex_config.parent().unwrap()).unwrap();
    fs::write(&roots.codex_config, b"model = \"native-model\"\n").unwrap();
    let (server, core) = verified_core(&fixture).await;
    mount_issue(&server, ClientKind::Codex).await;
    let manager = ClientIntegrationManager::new(
        roots,
        fixture.path().join("state"),
        fixture.path().join("bin").join("wokrouter"),
    )
    .unwrap();
    let management_token = SecretString::from(MANAGEMENT_TOKEN.to_owned());

    let (first, second) = tokio::join!(
        manager.inject(ClientKind::Codex, &core, &management_token),
        manager.inject(ClientKind::Codex, &core, &management_token)
    );

    assert_eq!(first.unwrap(), second.unwrap());
    assert!(matches!(
        manager.status(ClientKind::Codex).unwrap(),
        IntegrationStatus::Injected { .. }
    ));
}

#[tokio::test]
async fn reinjection_syncs_a_changed_wokcore_endpoint_without_rotating_the_token() {
    let fixture = tempdir().unwrap();
    let roots = client_roots(&fixture);
    fs::create_dir_all(roots.codex_config.parent().unwrap()).unwrap();
    let native = b"# native\nmodel = \"native-model\"\n";
    fs::write(&roots.codex_config, native).unwrap();
    let (first_server, core) = verified_core(&fixture).await;
    mount_issue(&first_server, ClientKind::Codex).await;
    let manager = ClientIntegrationManager::new(
        roots.clone(),
        fixture.path().join("state"),
        fixture.path().join("bin").join("wokrouter"),
    )
    .unwrap();
    let management_token = SecretString::from(MANAGEMENT_TOKEN.to_owned());

    manager
        .inject(ClientKind::Codex, &core, &management_token)
        .await
        .unwrap();
    let first_config = fs::read_to_string(&roots.codex_config).unwrap();
    assert!(first_config.contains(&format!("{}/v1/", first_server.uri())));

    let second_server = MockServer::start().await;
    mount_runtime(
        &second_server,
        &[
            "client_token.issue",
            "client_token.inspect",
            "client_token.revoke",
        ],
    )
    .await;
    write_discovery(&fixture, &second_server);
    mount_revoke(&second_server, ClientKind::Codex).await;

    let synced = manager
        .inject(ClientKind::Codex, &core, &management_token)
        .await
        .unwrap();

    assert!(matches!(synced, IntegrationStatus::Injected { .. }));
    let synced_config = fs::read_to_string(&roots.codex_config).unwrap();
    assert!(synced_config.contains(&format!("{}/v1/", second_server.uri())));
    assert!(!synced_config.contains(&first_server.uri()));
    assert_eq!(
        manager
            .restore(ClientKind::Codex, &core, &management_token)
            .await
            .unwrap(),
        RestoreResult::Restored
    );
    assert_eq!(fs::read(&roots.codex_config).unwrap(), native);
}

fn client_roots(fixture: &tempfile::TempDir) -> ClientRoots {
    let home = fixture.path().join("home");
    ClientRoots {
        home: home.clone(),
        codex_config: home.join(".codex").join("config.toml"),
        claude_settings: home.join(".claude").join("settings.json"),
        copilot_data: home.join(".copilot-app"),
    }
}

async fn verified_core(fixture: &tempfile::TempDir) -> (MockServer, WokCoreClient) {
    verified_core_with_capabilities(
        fixture,
        &[
            "client_token.issue",
            "client_token.inspect",
            "client_token.revoke",
        ],
    )
    .await
}

async fn verified_core_with_capabilities(
    fixture: &tempfile::TempDir,
    capabilities: &[&str],
) -> (MockServer, WokCoreClient) {
    let server = MockServer::start().await;
    mount_runtime(&server, capabilities).await;
    write_discovery(fixture, &server);
    let discovery = fixture.path().join("discovery.json");
    (server, WokCoreClient::new(discovery).unwrap())
}

async fn mount_runtime(server: &MockServer, capabilities: &[&str]) {
    mount_runtime_with_installation(
        server,
        capabilities,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .await;
}

async fn mount_runtime_with_installation(
    server: &MockServer,
    capabilities: &[&str],
    installation_id: &str,
) {
    mount_runtime_with_identity(server, capabilities, installation_id, INSTANCE_ID).await;
}

async fn mount_runtime_with_identity(
    server: &MockServer,
    capabilities: &[&str],
    installation_id: &str,
    instance_id: &str,
) {
    let authority = server.uri().trim_start_matches("http://").to_owned();
    Mock::given(method("GET"))
        .and(path("/wokcore/v1/health"))
        .and(header("host", authority.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "ok",
            "instance_id": instance_id
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
            "provider_protocols": [
                "anthropic.messages.v1",
                "openai.chat_completions.v1",
                "openai.responses.v1"
            ],
            "capabilities": capabilities,
            "instance_id": instance_id,
            "installation_id": installation_id
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(
            r"^/wokcore/v1/clients/[a-z0-9._-]+/tokens/[0-9a-f-]+$",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"active": true})))
        .mount(server)
        .await;
}

fn write_discovery(fixture: &tempfile::TempDir, server: &MockServer) {
    write_discovery_url(fixture, &server.uri(), INSTANCE_ID);
}

fn write_discovery_url(fixture: &tempfile::TempDir, base_url: &str, instance_id: &str) {
    let discovery = fixture.path().join("discovery.json");
    fs::write(
        &discovery,
        serde_json::to_vec(&json!({
            "base_url": base_url,
            "pid": std::process::id(),
            "instance_id": instance_id,
            "wokcore_version": "0.1.0",
            "api_major": 1
        }))
        .unwrap(),
    )
    .unwrap();
    secure_test_file(&discovery);
}

fn doctor_check<'a>(
    report: &'a wokrouter_platform::DoctorReport,
    id: &str,
) -> &'a wokrouter_platform::DoctorCheck {
    report.checks.iter().find(|check| check.id == id).unwrap()
}

async fn mount_issue(server: &MockServer, client: ClientKind) {
    mount_issue_times(server, client, 1).await;
}

async fn mount_issue_times(server: &MockServer, client: ClientKind, count: u64) {
    Mock::given(method("POST"))
        .and(path("/wokcore/v1/clients/authorize"))
        .respond_with(move |request: &wiremock::Request| {
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(body["client_id"], client.client_id());
            assert_eq!(body["scopes"], json!(["proxy.use"]));
            let token_id = body["token_id"].as_str().unwrap();
            ResponseTemplate::new(201).set_body_json(json!({
                "client_id": client.client_id(),
                "token_id": token_id,
                "token": CLIENT_TOKEN,
                "scopes": ["proxy.use"]
            }))
        })
        .expect(count)
        .mount(server)
        .await;
}

async fn mount_revoke(server: &MockServer, client: ClientKind) {
    Mock::given(method("DELETE"))
        .and(path_regex(format!(
            r"^/wokcore/v1/clients/{}/tokens/[0-9a-f-]+$",
            client.client_id().replace('.', r"\.")
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "revoked": true
        })))
        .expect(1)
        .mount(server)
        .await;
}

#[cfg(unix)]
fn secure_test_file(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[cfg(all(windows, feature = "test-support"))]
fn secure_test_file(path: &std::path::Path) {
    wokrouter_platform::test_support::secure_private_file(path).unwrap();
}

#[cfg(all(windows, not(feature = "test-support")))]
fn secure_test_file(_path: &std::path::Path) {}

#[cfg(not(any(unix, windows)))]
fn secure_test_file(_path: &std::path::Path) {}
