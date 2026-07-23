use std::{fs, path::Path};

use secrecy::{ExposeSecret, SecretString};
use wokrouter_core::{
    id::{AccountId, ProviderId},
    secret::{SecretPurpose, SecretRef, SecretScope},
};
use wokrouter_storage::{
    EnvironmentSecretStore, HeadlessSecretStoreConfig, MemorySecretStore,
    PermissionedFileSecretStore, SecretStore, StorageError,
};

fn scope() -> SecretScope {
    SecretScope {
        provider_id: ProviderId::new("openai").unwrap(),
        account_id: Some(AccountId::new("primary-account").unwrap()),
        purpose: SecretPurpose::ApiKey,
    }
}

#[test]
fn domain_identifiers_reject_unsafe_values() {
    assert!(ProviderId::new("").is_err());
    assert!(ProviderId::new("Open AI").is_err());
    assert!(AccountId::new("../account").is_err());
    assert!(ProviderId::new("openai-compatible.v1").is_ok());
}

#[tokio::test]
async fn secret_values_never_appear_in_debug_or_serialized_refs() {
    let store = MemorySecretStore::default();
    let plaintext = ["top", "secret"].join("-");
    let value = SecretString::from(plaintext.clone());

    let secret_ref = store.put(&scope(), value).await.unwrap();

    assert!(!format!("{secret_ref:?}").contains(&plaintext));
    assert!(
        !serde_json::to_string(&secret_ref)
            .unwrap()
            .contains(&plaintext)
    );
    assert!(store.get(&secret_ref).await.unwrap().expose_secret() == plaintext);
}

#[tokio::test]
async fn memory_store_round_trips_deletes_and_reports_missing_secrets() {
    let store = MemorySecretStore::default();
    let expected = ["memory", "value"].join("-");
    let secret_ref = store
        .put(&scope(), SecretString::from(expected.clone()))
        .await
        .unwrap();

    assert!(store.get(&secret_ref).await.unwrap().expose_secret() == expected);
    store.delete(&secret_ref).await.unwrap();
    assert!(matches!(
        store.get(&secret_ref).await,
        Err(StorageError::SecretNotFound)
    ));
    store.delete(&secret_ref).await.unwrap();
}

#[tokio::test]
async fn environment_store_reads_only_the_explicitly_configured_variable() {
    let configured_name = format!("WOKROUTER_TEST_SECRET_{}", std::process::id());
    let decoy_name = format!("{configured_name}_DECOY");
    let expected = ["configured", "value"].join("-");
    let secret_ref = SecretRef::new();
    unsafe {
        std::env::set_var(&configured_name, &expected);
        std::env::set_var(&decoy_name, "decoy-value");
    }
    let store = EnvironmentSecretStore::from_config(HeadlessSecretStoreConfig::Environment {
        secret_ref: secret_ref.clone(),
        variable_name: configured_name.clone(),
    })
    .unwrap();

    let value = store.get(&secret_ref).await.unwrap();

    unsafe {
        std::env::remove_var(configured_name);
        std::env::remove_var(decoy_name);
    }
    assert!(value.expose_secret() == expected);
    assert!(matches!(
        store.get(&SecretRef::new()).await,
        Err(StorageError::SecretNotFound)
    ));
}

#[test]
fn headless_stores_require_the_matching_explicit_configuration_variant() {
    let file_config = HeadlessSecretStoreConfig::PermissionedFile {
        secret_ref: SecretRef::new(),
        path: "secret.txt".into(),
    };
    let environment_config = HeadlessSecretStoreConfig::Environment {
        secret_ref: SecretRef::new(),
        variable_name: "WOKROUTER_SECRET".to_owned(),
    };

    assert!(EnvironmentSecretStore::from_config(file_config).is_err());
    assert!(PermissionedFileSecretStore::from_config(environment_config).is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn permissioned_file_store_rejects_modes_broader_than_owner_read_write() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("secret");
    let expected = ["file", "value"].join("-");
    fs::write(&path, &expected).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let secret_ref = SecretRef::new();
    let store =
        PermissionedFileSecretStore::from_config(HeadlessSecretStoreConfig::PermissionedFile {
            secret_ref: secret_ref.clone(),
            path: path.clone(),
        })
        .unwrap();

    assert!(matches!(
        store.get(&secret_ref).await,
        Err(StorageError::InsecureSecretFilePermissions)
    ));

    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(store.get(&secret_ref).await.unwrap().expose_secret() == expected);
}

#[cfg(windows)]
#[tokio::test]
async fn permissioned_file_store_rejects_acls_granting_other_principals() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("secret");
    fs::write(&path, ["file", "value"].join("-")).unwrap();
    let secret_ref = SecretRef::new();
    let store =
        PermissionedFileSecretStore::from_config(HeadlessSecretStoreConfig::PermissionedFile {
            secret_ref: secret_ref.clone(),
            path,
        })
        .unwrap();

    assert!(matches!(
        store.get(&secret_ref).await,
        Err(StorageError::InsecureSecretFilePermissions)
    ));
}

#[tokio::test]
async fn deleting_an_orphan_secret_leaves_no_plaintext_repository_artifact() {
    let store = MemorySecretStore::default();
    let plaintext = ["orphan", "fixture", "plaintext"].join("-");
    let secret_ref = store
        .put(&scope(), SecretString::from(plaintext.clone()))
        .await
        .unwrap();

    store.delete(&secret_ref).await.unwrap();

    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    assert_repository_files_do_not_contain(repository, plaintext.as_bytes());
}

fn assert_repository_files_do_not_contain(directory: &Path, needle: &[u8]) {
    for entry in fs::read_dir(directory).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let name = entry.file_name();
        if name == ".git" || name == "target" || name == ".worktrees" {
            continue;
        }
        if path.is_dir() {
            assert_repository_files_do_not_contain(&path, needle);
        } else if matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("rs" | "toml" | "sql" | "md" | "json")
        ) {
            assert!(
                !fs::read(&path)
                    .unwrap()
                    .windows(needle.len())
                    .any(|window| window == needle),
                "plaintext secret found in repository file {}",
                path.display()
            );
        }
    }
}
