use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Component, Path, PathBuf},
};

use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::atomic_edit::{
    create_private_directory, private_file, remove_private_file, replace_private_file,
    secure_existing_file, write_new_private_file,
};

const SCHEMA_VERSION: u32 = 1;
const MAX_MUTATION_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct MutationJournal {
    root: PathBuf,
    allowed_root: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MutationId(String);

impl MutationId {
    pub(super) fn is_valid(&self) -> bool {
        Uuid::parse_str(&self.0).is_ok_and(|id| id.to_string() == self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationOperation {
    CodexConfig,
    ClaudeConfig,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationStatus {
    Prepared,
    Committed,
    Restored,
    Conflict,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnedMutation {
    pub id: MutationId,
    pub operation: MutationOperation,
    pub status: MutationStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RestoreResult {
    Restored,
    AlreadyRestored,
    Conflict { recovery_path: PathBuf },
    ManualActionRequired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MutationError {
    #[error("client mutation I/O failed")]
    Io,
    #[error("client mutation target is outside the configured fake-home or user-home root")]
    UnsafeTarget,
    #[error("client mutation record is invalid")]
    InvalidRecord,
    #[error("client mutation is not supported on this platform")]
    UnsupportedPlatform,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MutationRecord {
    schema_version: u32,
    id: MutationId,
    operation: MutationOperation,
    status: MutationStatus,
    target: PathBuf,
    before_hash: Option<String>,
    after_hash: String,
    backup_name: Option<String>,
    recovery_name: Option<String>,
    committed_at: Option<String>,
}

pub struct PreparedMutation<'a> {
    journal: &'a MutationJournal,
    record: MutationRecord,
    replacement: Vec<u8>,
    applied: bool,
    _lock: File,
}

impl MutationJournal {
    pub fn new(
        root: impl Into<PathBuf>,
        allowed_root: impl AsRef<Path>,
    ) -> Result<Self, MutationError> {
        let root = absolute_path(root.into())?;
        let allowed_root = absolute_path(allowed_root.as_ref().to_path_buf())?;
        create_private_directory(&root)?;
        fs::create_dir_all(&allowed_root).map_err(|_| MutationError::Io)?;
        if fs::symlink_metadata(&allowed_root)
            .map_err(|_| MutationError::Io)?
            .file_type()
            .is_symlink()
        {
            return Err(MutationError::UnsafeTarget);
        }
        let allowed_root = fs::canonicalize(allowed_root).map_err(|_| MutationError::Io)?;
        let journal = Self { root, allowed_root };
        journal.recover_prepared()?;
        Ok(journal)
    }

    pub fn replace(
        &self,
        target: &Path,
        replacement: &[u8],
        operation: MutationOperation,
    ) -> Result<OwnedMutation, MutationError> {
        let mut pending = self.begin(target, replacement, operation)?;
        pending.apply()?;
        pending.commit()
    }

    pub fn begin(
        &self,
        target: &Path,
        replacement: &[u8],
        operation: MutationOperation,
    ) -> Result<PreparedMutation<'_>, MutationError> {
        if replacement.len() > MAX_MUTATION_BYTES {
            return Err(MutationError::InvalidRecord);
        }
        let lock = self.lock()?;
        self.recover_prepared_locked()?;
        let target = self.validated_target(target)?;
        let before = read_optional_bounded(&target)?;
        let id = MutationId(Uuid::new_v4().to_string());
        let backup_name = before.as_ref().map(|_| format!("{}.before", id.0.as_str()));
        if let (Some(bytes), Some(name)) = (&before, &backup_name) {
            write_new_private_file(&self.root.join(name), bytes)?;
        }
        let record = MutationRecord {
            schema_version: SCHEMA_VERSION,
            id,
            operation,
            status: MutationStatus::Prepared,
            target,
            before_hash: before.as_deref().map(content_hash),
            after_hash: content_hash(replacement),
            backup_name,
            recovery_name: None,
            committed_at: None,
        };
        self.write_record(&record)?;
        Ok(PreparedMutation {
            journal: self,
            record,
            replacement: replacement.to_vec(),
            applied: false,
            _lock: lock,
        })
    }

    pub fn restore(&self, id: &MutationId) -> Result<RestoreResult, MutationError> {
        let _lock = self.lock()?;
        self.recover_prepared_locked()?;
        let mut record = self.read_record(id)?;
        match record.status {
            MutationStatus::Restored => return Ok(RestoreResult::AlreadyRestored),
            MutationStatus::Conflict => {
                return Ok(RestoreResult::Conflict {
                    recovery_path: self.recovery_path(&record)?,
                });
            }
            MutationStatus::Prepared => return Err(MutationError::InvalidRecord),
            MutationStatus::Committed => {}
        }
        let current = read_optional_bounded(&record.target)?;
        let current_hash = current.as_deref().map(content_hash);
        if current_hash == record.before_hash {
            record.status = MutationStatus::Restored;
            self.write_record(&record)?;
            return Ok(RestoreResult::AlreadyRestored);
        }
        if current_hash.as_deref() != Some(record.after_hash.as_str()) {
            let recovery_path = self.mark_conflict(&mut record, current_hash)?;
            return Ok(RestoreResult::Conflict { recovery_path });
        }
        self.restore_before(&record)?;
        record.status = MutationStatus::Restored;
        self.write_record(&record)?;
        Ok(RestoreResult::Restored)
    }

    pub fn recover_prepared(&self) -> Result<usize, MutationError> {
        let _lock = self.lock()?;
        self.recover_prepared_locked()
    }

    fn recover_prepared_locked(&self) -> Result<usize, MutationError> {
        let mut recovered = 0;
        for entry in fs::read_dir(&self.root).map_err(|_| MutationError::Io)? {
            let entry = entry.map_err(|_| MutationError::Io)?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json")
                || path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| name.ends_with(".conflict.json"))
            {
                continue;
            }
            let mut record = read_record_path(&path)?;
            if path != self.record_path(&record.id) {
                return Err(MutationError::InvalidRecord);
            }
            self.validate_loaded_record(&record)?;
            if record.status != MutationStatus::Prepared {
                continue;
            }
            let current = read_optional_bounded(&record.target)?;
            let current_hash = current.as_deref().map(content_hash);
            if current_hash.as_deref() == Some(record.after_hash.as_str()) {
                self.restore_before(&record)?;
                record.status = MutationStatus::Restored;
                self.write_record(&record)?;
            } else if current_hash == record.before_hash {
                record.status = MutationStatus::Restored;
                self.write_record(&record)?;
            } else {
                self.mark_conflict(&mut record, current_hash)?;
            }
            recovered += 1;
        }
        Ok(recovered)
    }

    fn validated_target(&self, target: &Path) -> Result<PathBuf, MutationError> {
        let target = absolute_path(target.to_path_buf())?;
        let parent = target.parent().ok_or(MutationError::UnsafeTarget)?;
        let parent = fs::canonicalize(parent).map_err(|_| MutationError::UnsafeTarget)?;
        if !parent.starts_with(&self.allowed_root) {
            return Err(MutationError::UnsafeTarget);
        }
        let mut current = Some(parent.as_path());
        while let Some(path) = current {
            let metadata = fs::symlink_metadata(path).map_err(|_| MutationError::UnsafeTarget)?;
            if metadata.file_type().is_symlink() {
                return Err(MutationError::UnsafeTarget);
            }
            if path == self.allowed_root {
                break;
            }
            current = path.parent();
        }
        if current.is_none() {
            return Err(MutationError::UnsafeTarget);
        }
        if fs::symlink_metadata(&target)
            .is_ok_and(|metadata| metadata.file_type().is_symlink() || !metadata.is_file())
        {
            return Err(MutationError::UnsafeTarget);
        }
        let file_name = target.file_name().ok_or(MutationError::UnsafeTarget)?;
        Ok(parent.join(file_name))
    }

    fn lock(&self) -> Result<File, MutationError> {
        let path = self.root.join("journal.lock");
        if fs::symlink_metadata(&path)
            .is_ok_and(|metadata| metadata.file_type().is_symlink() || !metadata.is_file())
        {
            return Err(MutationError::InvalidRecord);
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|_| MutationError::Io)?;
        secure_existing_file(&path)?;
        file.lock_exclusive().map_err(|_| MutationError::Io)?;
        Ok(file)
    }

    fn write_record(&self, record: &MutationRecord) -> Result<(), MutationError> {
        let bytes = serde_json::to_vec(record).map_err(|_| MutationError::InvalidRecord)?;
        replace_private_file(&self.record_path(&record.id), &bytes)
    }

    fn read_record(&self, id: &MutationId) -> Result<MutationRecord, MutationError> {
        let record = read_record_path(&self.record_path(id))?;
        if &record.id != id {
            return Err(MutationError::InvalidRecord);
        }
        self.validate_loaded_record(&record)?;
        Ok(record)
    }

    fn record_path(&self, id: &MutationId) -> PathBuf {
        self.root.join(format!("{}.json", id.0))
    }

    fn restore_before(&self, record: &MutationRecord) -> Result<(), MutationError> {
        match &record.backup_name {
            Some(name) => {
                let path = self.root.join(name);
                if !private_file(&path) {
                    return Err(MutationError::InvalidRecord);
                }
                let bytes = read_bounded(&path)?;
                if Some(content_hash(&bytes)) != record.before_hash {
                    return Err(MutationError::InvalidRecord);
                }
                replace_private_file(&record.target, &bytes)
            }
            None => remove_private_file(&record.target),
        }
    }

    fn mark_conflict(
        &self,
        record: &mut MutationRecord,
        current_hash: Option<String>,
    ) -> Result<PathBuf, MutationError> {
        let recovery_name = format!("{}.conflict.json", record.id.0);
        let recovery_path = self.root.join(&recovery_name);
        let recovery = ConflictManifest {
            schema_version: SCHEMA_VERSION,
            mutation_id: &record.id,
            operation: record.operation,
            before_hash: record.before_hash.as_deref(),
            expected_hash: &record.after_hash,
            current_hash: current_hash.as_deref(),
        };
        let bytes = serde_json::to_vec(&recovery).map_err(|_| MutationError::InvalidRecord)?;
        replace_private_file(&recovery_path, &bytes)?;
        record.status = MutationStatus::Conflict;
        record.recovery_name = Some(recovery_name);
        self.write_record(record)?;
        Ok(recovery_path)
    }

    fn recovery_path(&self, record: &MutationRecord) -> Result<PathBuf, MutationError> {
        record
            .recovery_name
            .as_ref()
            .map(|name| self.root.join(name))
            .ok_or(MutationError::InvalidRecord)
    }

    fn validate_loaded_record(&self, record: &MutationRecord) -> Result<(), MutationError> {
        let target = self
            .validated_target(&record.target)
            .map_err(|_| MutationError::InvalidRecord)?;
        let expected_backup = record
            .before_hash
            .as_ref()
            .map(|_| format!("{}.before", record.id.0));
        let expected_recovery = (record.status == MutationStatus::Conflict)
            .then(|| format!("{}.conflict.json", record.id.0));
        if target != record.target
            || record.backup_name != expected_backup
            || record.recovery_name != expected_recovery
        {
            return Err(MutationError::InvalidRecord);
        }
        Ok(())
    }
}

impl PreparedMutation<'_> {
    pub fn id(&self) -> &MutationId {
        &self.record.id
    }

    pub fn apply(&mut self) -> Result<(), MutationError> {
        replace_private_file(&self.record.target, &self.replacement)?;
        self.applied = true;
        Ok(())
    }

    pub fn commit(mut self) -> Result<OwnedMutation, MutationError> {
        let current_hash = read_optional_bounded(&self.record.target)?
            .as_deref()
            .map(content_hash);
        if !self.applied || current_hash.as_deref() != Some(self.record.after_hash.as_str()) {
            return Err(MutationError::InvalidRecord);
        }
        self.record.status = MutationStatus::Committed;
        self.record.committed_at = Some(jiff::Timestamp::now().to_string());
        self.journal.write_record(&self.record)?;
        Ok(OwnedMutation {
            id: self.record.id.clone(),
            operation: self.record.operation,
            status: self.record.status,
        })
    }
}

#[derive(Serialize)]
struct ConflictManifest<'a> {
    schema_version: u32,
    mutation_id: &'a MutationId,
    operation: MutationOperation,
    before_hash: Option<&'a str>,
    expected_hash: &'a str,
    current_hash: Option<&'a str>,
}

fn absolute_path(path: PathBuf) -> Result<PathBuf, MutationError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(MutationError::UnsafeTarget);
    }
    Ok(path)
}

fn read_record_path(path: &Path) -> Result<MutationRecord, MutationError> {
    if !private_file(path) {
        return Err(MutationError::InvalidRecord);
    }
    let bytes = read_bounded(path)?;
    let record: MutationRecord =
        serde_json::from_slice(&bytes).map_err(|_| MutationError::InvalidRecord)?;
    if record.schema_version != SCHEMA_VERSION
        || !Uuid::parse_str(&record.id.0).is_ok_and(|id| id.to_string() == record.id.0)
        || !valid_hash(&record.after_hash)
        || record
            .before_hash
            .as_ref()
            .is_some_and(|hash| !valid_hash(hash))
    {
        return Err(MutationError::InvalidRecord);
    }
    Ok(record)
}

fn read_optional_bounded(path: &Path) -> Result<Option<Vec<u8>>, MutationError> {
    match File::open(path) {
        Ok(mut file) => {
            let metadata = file.metadata().map_err(|_| MutationError::Io)?;
            if !metadata.is_file() || metadata.len() > MAX_MUTATION_BYTES as u64 {
                return Err(MutationError::InvalidRecord);
            }
            let mut bytes = Vec::with_capacity(metadata.len() as usize);
            file.by_ref()
                .take((MAX_MUTATION_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|_| MutationError::Io)?;
            if bytes.len() > MAX_MUTATION_BYTES {
                return Err(MutationError::InvalidRecord);
            }
            Ok(Some(bytes))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(MutationError::Io),
    }
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, MutationError> {
    read_optional_bounded(path)?.ok_or(MutationError::InvalidRecord)
}

fn content_hash(contents: &[u8]) -> String {
    format!("{:x}", Sha256::digest(contents))
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
