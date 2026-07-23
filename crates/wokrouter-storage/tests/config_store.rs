use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Barrier},
    thread,
    time::{Duration, Instant},
};

use wokrouter_storage::{AppConfig, ConfigStore, StorageError};

#[test]
fn commit_requires_current_revision_and_survives_reload() {
    let directory = tempfile::tempdir().unwrap();
    let store = ConfigStore::new(directory.path().join("config.toml"));

    let initial = store.load().unwrap();
    assert_eq!(initial.revision, 0);
    assert_eq!(initial.config, AppConfig::default());

    let mut candidate = initial.config.clone();
    candidate.server.port = 10101;
    let committed = store.commit(0, &candidate).unwrap();
    assert_eq!(committed.revision, 1);
    assert_eq!(store.load().unwrap(), committed);

    let error = store.commit(0, &candidate).unwrap_err();
    assert!(matches!(
        error,
        StorageError::RevisionConflict {
            expected: 0,
            actual: 1
        }
    ));
}

#[test]
fn malformed_toml_is_reported_without_overwriting_the_source() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    let malformed = "revision = [\n";
    fs::write(&path, malformed).unwrap();

    let error = ConfigStore::new(&path)
        .commit(0, &AppConfig::default())
        .unwrap_err();

    assert!(matches!(error, StorageError::InvalidConfig { .. }));
    assert_eq!(fs::read_to_string(path).unwrap(), malformed);
}

#[test]
fn commit_atomically_replaces_an_existing_config() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    let store = ConfigStore::new(&path);

    let first = store.commit(0, &AppConfig::default()).unwrap();
    let mut replacement = first.config.clone();
    replacement.server.port = 10102;

    let second = store.commit(first.revision, &replacement).unwrap();

    assert_eq!(second.revision, 2);
    assert_eq!(store.load().unwrap(), second);
    let entries = fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(entries, vec!["config.toml", "config.toml.lock"]);
}

#[test]
fn concurrent_commits_with_the_same_revision_allow_only_one_success() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    let store = Arc::new(ConfigStore::new(path));
    let barrier = Arc::new(Barrier::new(2));
    let mut candidate = AppConfig::default();
    candidate.ui.locale_override = Some("x".repeat(16 * 1024 * 1024));

    let first = {
        let barrier = Arc::clone(&barrier);
        let candidate = candidate.clone();
        let store = Arc::clone(&store);
        thread::spawn(move || {
            barrier.wait();
            store.commit(0, &candidate)
        })
    };
    let second = {
        let barrier = Arc::clone(&barrier);
        let store = Arc::clone(&store);
        thread::spawn(move || {
            barrier.wait();
            store.commit(0, &candidate)
        })
    };

    let results = [first.join().unwrap(), second.join().unwrap()];
    assert_eq!(
        results.iter().filter(|result| result.is_ok()).count(),
        1,
        "only one commit may claim the same revision"
    );
    assert!(results.iter().any(|result| matches!(
        result,
        Err(StorageError::RevisionConflict {
            expected: 0,
            actual: 1
        })
    )));
}

#[test]
fn concurrent_process_commits_with_the_same_revision_allow_only_one_success() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    let synchronization = directory.path().join("synchronization");
    fs::create_dir(&synchronization).unwrap();
    let executable = env::current_exe().unwrap();

    let mut children = ["first", "second"].map(|id| {
        Command::new(&executable)
            .args(["--exact", "process_commit_helper", "--nocapture"])
            .env("WOKROUTER_STORAGE_HELPER_CONFIG", &path)
            .env("WOKROUTER_STORAGE_HELPER_SYNC", &synchronization)
            .env("WOKROUTER_STORAGE_HELPER_ID", id)
            .spawn()
            .unwrap()
    });

    wait_for_all(&synchronization, &["first.ready", "second.ready"]);
    fs::write(synchronization.join("start"), []).unwrap();

    for child in &mut children {
        assert!(child.wait().unwrap().success());
    }

    let results =
        ["first", "second"].map(|id| fs::read_to_string(synchronization.join(id)).unwrap());
    assert_eq!(
        results.iter().filter(|result| *result == "success").count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| *result == "conflict")
            .count(),
        1
    );
}

#[test]
fn process_commit_helper() {
    let Ok(config_path) = env::var("WOKROUTER_STORAGE_HELPER_CONFIG") else {
        return;
    };
    let synchronization = PathBuf::from(env::var("WOKROUTER_STORAGE_HELPER_SYNC").unwrap());
    let identifier = env::var("WOKROUTER_STORAGE_HELPER_ID").unwrap();
    fs::write(synchronization.join(format!("{identifier}.ready")), []).unwrap();
    wait_for_all(&synchronization, &["start"]);

    let mut candidate = AppConfig::default();
    candidate.ui.locale_override = Some("x".repeat(16 * 1024 * 1024));
    let result = ConfigStore::new(config_path).commit(0, &candidate);
    let outcome = match result {
        Ok(_) => "success",
        Err(StorageError::RevisionConflict {
            expected: 0,
            actual: 1,
        }) => "conflict",
        Err(error) => panic!("unexpected commit result: {error}"),
    };
    fs::write(synchronization.join(identifier), outcome).unwrap();
}

fn wait_for_all(directory: &Path, names: &[&str]) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !names.iter().all(|name| directory.join(name).exists()) {
        assert!(Instant::now() < deadline, "timed out waiting for {names:?}");
        thread::sleep(Duration::from_millis(10));
    }
}
