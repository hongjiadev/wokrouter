use std::fs;

use tempfile::tempdir;
use wokrouter_platform::{
    MutationError, MutationJournal, MutationOperation, MutationStatus, RestoreResult,
};

#[test]
fn committed_mutations_restore_exactly_and_idempotently() {
    let fixture = tempdir().unwrap();
    let home = fixture.path().join("home");
    let state = fixture.path().join("state");
    let target = home.join(".codex").join("config.toml");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, "# native\r\nmodel = \"native\"\r\n").unwrap();
    let journal = MutationJournal::new(state.join("journal"), &home).unwrap();

    let mutation = journal
        .replace(
            &target,
            b"# native\r\nmodel = \"wokcore\"\r\n",
            MutationOperation::CodexConfig,
        )
        .unwrap();
    assert_eq!(mutation.status, MutationStatus::Committed);

    assert_eq!(
        journal.restore(&mutation.id).unwrap(),
        RestoreResult::Restored
    );
    assert_eq!(
        fs::read(&target).unwrap(),
        b"# native\r\nmodel = \"native\"\r\n"
    );
    assert_eq!(
        journal.restore(&mutation.id).unwrap(),
        RestoreResult::AlreadyRestored
    );
}

#[test]
fn restore_never_overwrites_a_user_edit_and_conflict_manifest_has_no_contents() {
    let fixture = tempdir().unwrap();
    let home = fixture.path().join("home");
    let state = fixture.path().join("state");
    let target = home.join(".claude").join("settings.json");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, br#"{"nativeSecret":"do-not-copy"}"#).unwrap();
    let journal = MutationJournal::new(state.join("journal"), &home).unwrap();
    let mutation = journal
        .replace(
            &target,
            br#"{"apiKeyHelper":"wokrouter integration-token claude"}"#,
            MutationOperation::ClaudeConfig,
        )
        .unwrap();
    fs::write(&target, br#"{"userEdit":true,"token":"user-secret"}"#).unwrap();

    let result = journal.restore(&mutation.id).unwrap();
    let RestoreResult::Conflict { recovery_path } = result else {
        panic!("expected a conflict");
    };

    assert_eq!(
        fs::read(&target).unwrap(),
        br#"{"userEdit":true,"token":"user-secret"}"#
    );
    let recovery = fs::read_to_string(recovery_path).unwrap();
    assert!(!recovery.contains("do-not-copy"));
    assert!(!recovery.contains("user-secret"));
    assert!(!recovery.contains("apiKeyHelper"));
}

#[test]
fn startup_recovery_rolls_back_a_prepared_mutation_after_replace() {
    let fixture = tempdir().unwrap();
    let home = fixture.path().join("home");
    let state = fixture.path().join("state");
    let target = home.join(".codex").join("config.toml");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, b"native = true\n").unwrap();
    let journal = MutationJournal::new(state.join("journal"), &home).unwrap();
    let mut pending = journal
        .begin(
            &target,
            b"native = true\nwokcore = true\n",
            MutationOperation::CodexConfig,
        )
        .unwrap();
    pending.apply().unwrap();
    drop(pending);

    let recovered = journal.recover_prepared().unwrap();

    assert_eq!(recovered, 1);
    assert_eq!(fs::read(&target).unwrap(), b"native = true\n");
}

#[test]
fn restore_rejects_a_record_whose_target_was_moved_outside_the_allowed_home() {
    let fixture = tempdir().unwrap();
    let home = fixture.path().join("home");
    let state = fixture.path().join("state");
    let target = home.join(".codex").join("config.toml");
    let outside = fixture.path().join("outside.toml");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, b"native = true\n").unwrap();
    fs::write(&outside, b"wokcore = true\n").unwrap();
    let journal = MutationJournal::new(state.join("journal"), &home).unwrap();
    let mutation = journal
        .replace(&target, b"wokcore = true\n", MutationOperation::CodexConfig)
        .unwrap();
    let record_path = fs::read_dir(state.join("journal"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension().and_then(|value| value.to_str()) == Some("json")
                && !path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .ends_with(".conflict.json")
        })
        .unwrap();
    let mut record: serde_json::Value =
        serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    record["target"] = serde_json::Value::String(outside.to_string_lossy().into_owned());
    fs::write(&record_path, serde_json::to_vec(&record).unwrap()).unwrap();

    assert_eq!(
        journal.restore(&mutation.id).unwrap_err(),
        MutationError::InvalidRecord
    );
    assert_eq!(fs::read(&outside).unwrap(), b"wokcore = true\n");
}
