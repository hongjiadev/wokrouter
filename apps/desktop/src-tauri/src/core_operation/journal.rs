use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use fs4::fs_std::FileExt;
use uuid::Uuid;
use wokrouter_platform::{
    is_private_directory, is_private_file, secure_private_directory, secure_private_file,
    system::private_paths::{PrivateDirectoryGuard, pin_private_directory},
};

use super::{CoreOperationError, CoreOperationSnapshot};

const JOURNAL_DIRECTORY: &str = "core-operation";
const JOURNAL_FILE: &str = "operation.json";
const CLAIM_FILE: &str = "claim.lock";
const LEASE_FILE: &str = "operation.lock";
const RECORD_LOCK_FILE: &str = "record.lock";
const MAX_JOURNAL_BYTES: usize = 16 * 1024;

pub(super) struct OperationJournal {
    root: PathBuf,
    record: PathBuf,
    directory_guard: PrivateDirectoryGuard,
}

pub(super) enum LeaseAttempt {
    Acquired(JournalLease),
    Busy,
}

pub(super) struct JournalLease {
    file: File,
}

impl Drop for JournalLease {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

impl OperationJournal {
    pub(super) fn open(runtime_directory: &Path) -> Result<Self, CoreOperationError> {
        let private_boundary = runtime_directory
            .parent()
            .ok_or(CoreOperationError::Initialization)?;
        fs::create_dir_all(private_boundary).map_err(|_| CoreOperationError::Initialization)?;
        secure_directory(private_boundary)?;
        let boundary_guard = pin_private_directory(private_boundary, private_boundary)
            .map_err(|_| CoreOperationError::Initialization)?;
        fs::create_dir_all(runtime_directory).map_err(|_| CoreOperationError::Initialization)?;
        secure_directory(runtime_directory)?;
        let runtime_guard = pin_private_directory(runtime_directory, private_boundary)
            .map_err(|_| CoreOperationError::Initialization)?;
        let root = runtime_directory.join(JOURNAL_DIRECTORY);
        fs::create_dir_all(&root).map_err(|_| CoreOperationError::Initialization)?;
        secure_directory(&root)?;
        let directory_guard = pin_private_directory(&root, private_boundary)
            .map_err(|_| CoreOperationError::Initialization)?;
        drop(boundary_guard);
        drop(runtime_guard);
        Ok(Self {
            record: root.join(JOURNAL_FILE),
            root,
            directory_guard,
        })
    }

    pub(super) fn read(&self) -> Result<Option<CoreOperationSnapshot>, CoreOperationError> {
        self.ensure_stable_root()?;
        let _record_lock = self.lock_record_shared()?;
        self.ensure_stable_root()?;
        let mut file = match File::open(&self.record) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(CoreOperationError::InvalidProgress),
        };
        if !is_private_file(&self.record) {
            return Err(CoreOperationError::InvalidProgress);
        }
        let metadata = file
            .metadata()
            .map_err(|_| CoreOperationError::InvalidProgress)?;
        if !metadata.is_file() || metadata.len() > MAX_JOURNAL_BYTES as u64 {
            return Err(CoreOperationError::InvalidProgress);
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        Read::by_ref(&mut file)
            .take((MAX_JOURNAL_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| CoreOperationError::InvalidProgress)?;
        if bytes.len() > MAX_JOURNAL_BYTES {
            return Err(CoreOperationError::InvalidProgress);
        }
        let value = serde_json::from_slice::<serde_json::Value>(&bytes)
            .map_err(|_| CoreOperationError::InvalidProgress)?;
        let encoded_id = value
            .as_object()
            .and_then(|object| object.get("operation_id"))
            .and_then(serde_json::Value::as_str)
            .ok_or(CoreOperationError::InvalidProgress)?;
        let parsed_id =
            Uuid::parse_str(encoded_id).map_err(|_| CoreOperationError::InvalidProgress)?;
        if parsed_id.to_string() != encoded_id {
            return Err(CoreOperationError::InvalidProgress);
        }
        let snapshot = serde_json::from_value::<CoreOperationSnapshot>(value)
            .map_err(|_| CoreOperationError::InvalidProgress)?;
        if !snapshot.is_safe_projection() {
            return Err(CoreOperationError::InvalidProgress);
        }
        Ok(Some(snapshot))
    }

    pub(super) fn write(&self, snapshot: &CoreOperationSnapshot) -> Result<(), CoreOperationError> {
        self.ensure_stable_root()?;
        if !snapshot.is_safe_projection() {
            return Err(CoreOperationError::InvalidProgress);
        }
        let bytes =
            serde_json::to_vec(snapshot).map_err(|_| CoreOperationError::InvalidProgress)?;
        if bytes.len() > MAX_JOURNAL_BYTES {
            return Err(CoreOperationError::InvalidProgress);
        }
        let _record_lock = self.lock_record_exclusive()?;
        self.ensure_stable_root()?;
        let temporary = self.root.join(format!("operation.{}.tmp", Uuid::new_v4()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        configure_private_create(&mut options);
        let mut file = options
            .open(&temporary)
            .map_err(|_| CoreOperationError::Initialization)?;
        secure_private_file(&temporary).map_err(|_| CoreOperationError::Initialization)?;
        let result = file
            .write_all(&bytes)
            .and_then(|()| file.sync_all())
            .and_then(|()| replace_file(&temporary, &self.record))
            .and_then(|()| secure_private_file(&self.record))
            .and_then(|()| sync_directory(&self.root));
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
            return Err(CoreOperationError::Initialization);
        }
        Ok(())
    }

    pub(super) fn try_claim(&self) -> Result<LeaseAttempt, CoreOperationError> {
        self.try_lock(CLAIM_FILE)
    }

    pub(super) fn try_operation_lease(&self) -> Result<LeaseAttempt, CoreOperationError> {
        self.try_lock(LEASE_FILE)
    }

    pub(super) fn operation_lease_active(&self) -> Result<bool, CoreOperationError> {
        match self.try_operation_lease()? {
            LeaseAttempt::Acquired(lease) => {
                drop(lease);
                Ok(false)
            }
            LeaseAttempt::Busy => Ok(true),
        }
    }

    fn try_lock(&self, name: &str) -> Result<LeaseAttempt, CoreOperationError> {
        let file = self.open_lock_file(name)?;
        match file.try_lock_exclusive() {
            Ok(true) => Ok(LeaseAttempt::Acquired(JournalLease { file })),
            Ok(false) => Ok(LeaseAttempt::Busy),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(LeaseAttempt::Busy),
            Err(_) => Err(CoreOperationError::Initialization),
        }
    }

    fn lock_record_shared(&self) -> Result<JournalLease, CoreOperationError> {
        let file = self.open_lock_file(RECORD_LOCK_FILE)?;
        file.lock_shared()
            .map_err(|_| CoreOperationError::Initialization)?;
        Ok(JournalLease { file })
    }

    fn lock_record_exclusive(&self) -> Result<JournalLease, CoreOperationError> {
        let file = self.open_lock_file(RECORD_LOCK_FILE)?;
        file.lock_exclusive()
            .map_err(|_| CoreOperationError::Initialization)?;
        Ok(JournalLease { file })
    }

    fn open_lock_file(&self, name: &str) -> Result<File, CoreOperationError> {
        self.ensure_stable_root()?;
        let path = self.root.join(name);
        if path.exists() && !is_private_file(&path) {
            return Err(CoreOperationError::InvalidProgress);
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        configure_private_create(&mut options);
        let file = options
            .open(&path)
            .map_err(|_| CoreOperationError::Initialization)?;
        secure_private_file(&path).map_err(|_| CoreOperationError::Initialization)?;
        if !is_private_file(&path) {
            return Err(CoreOperationError::InvalidProgress);
        }
        Ok(file)
    }

    fn ensure_stable_root(&self) -> Result<(), CoreOperationError> {
        self.directory_guard
            .verify()
            .then_some(())
            .ok_or(CoreOperationError::InvalidProgress)
    }
}

fn secure_directory(path: &Path) -> Result<(), CoreOperationError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| CoreOperationError::Initialization)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CoreOperationError::Initialization);
    }
    secure_private_directory(path).map_err(|_| CoreOperationError::Initialization)?;
    is_private_directory(path)
        .then_some(())
        .ok_or(CoreOperationError::Initialization)
}

#[cfg(unix)]
fn configure_private_create(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;

    options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
}

#[cfg(windows)]
fn configure_private_create(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;

    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

    options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
}

#[cfg(not(any(unix, windows)))]
fn configure_private_create(_options: &mut OpenOptions) {}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source = wide_path(source)?;
    let destination = wide_path(destination)?;
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn wide_path(path: &Path) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;

    const BACKSLASH: u16 = b'\\' as u16;
    const FORWARD_SLASH: u16 = b'/' as u16;
    const VERBATIM_PREFIX: [u16; 4] = [BACKSLASH, BACKSLASH, b'?' as u16, BACKSLASH];
    const DEVICE_PREFIX: [u16; 4] = [BACKSLASH, BACKSLASH, b'.' as u16, BACKSLASH];
    const VERBATIM_UNC_PREFIX: [u16; 8] = [
        BACKSLASH,
        BACKSLASH,
        b'?' as u16,
        BACKSLASH,
        b'U' as u16,
        b'N' as u16,
        b'C' as u16,
        BACKSLASH,
    ];

    let absolute = std::path::absolute(path)?;
    let mut encoded = absolute.as_os_str().encode_wide().collect::<Vec<_>>();
    if encoded.contains(&0) || encoded.starts_with(&DEVICE_PREFIX) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid Windows path",
        ));
    }
    for unit in &mut encoded {
        if *unit == FORWARD_SLASH {
            *unit = BACKSLASH;
        }
    }
    let mut verbatim = if encoded.starts_with(&VERBATIM_PREFIX) {
        encoded
    } else if encoded.starts_with(&[BACKSLASH, BACKSLASH]) {
        let mut value = VERBATIM_UNC_PREFIX.to_vec();
        value.extend_from_slice(&encoded[2..]);
        value
    } else {
        let mut value = VERBATIM_PREFIX.to_vec();
        value.extend(encoded);
        value
    };
    verbatim.push(0);
    Ok(verbatim)
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use std::{
        fs,
        process::{Command, Stdio},
    };

    use tempfile::tempdir;

    use super::{CoreOperationError, OperationJournal};

    #[test]
    fn journal_rejects_a_reparse_runtime_root() {
        let fixture = tempdir().unwrap();
        let target = fixture.path().join("target");
        let runtime = fixture.path().join("runtime");
        fs::create_dir(&target).unwrap();
        create_directory_junction(&target, &runtime);

        assert!(matches!(
            OperationJournal::open(&runtime),
            Err(CoreOperationError::Initialization)
        ));

        fs::remove_dir(&runtime).unwrap();
    }

    #[test]
    fn journal_fails_closed_after_a_real_lock_domain_swap() {
        let fixture = tempdir().unwrap();
        let runtime = fixture.path().join("runtime");
        let journal = OperationJournal::open(&runtime).unwrap();
        let root = runtime.join("core-operation");
        let displaced = runtime.join("displaced-core-operation");

        fs::rename(&root, &displaced).unwrap();
        let replacement = OperationJournal::open(&runtime).unwrap();

        assert!(matches!(
            journal.try_operation_lease(),
            Err(CoreOperationError::InvalidProgress)
        ));
        assert!(matches!(
            replacement.try_operation_lease(),
            Ok(super::LeaseAttempt::Acquired(_))
        ));
    }

    #[test]
    fn atomic_replace_supports_a_journal_path_beyond_max_path() {
        use std::os::windows::ffi::OsStrExt;

        let fixture = tempdir().unwrap();
        let mut directory = fixture.path().to_path_buf();
        for index in 0..6 {
            directory.push(format!("segment-{index}-{}", "x".repeat(48)));
        }
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("operation.pending.tmp");
        let destination = directory.join("operation.json");
        assert!(
            destination.as_os_str().encode_wide().count() > 260,
            "fixture did not exceed MAX_PATH"
        );
        fs::write(&source, b"new snapshot").unwrap();
        fs::write(&destination, b"old snapshot").unwrap();

        super::replace_file(&source, &destination).unwrap();

        assert_eq!(fs::read(&destination).unwrap(), b"new snapshot");
        assert!(!source.exists());
    }

    fn create_directory_junction(target: &std::path::Path, link: &std::path::Path) {
        let status = Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/j"])
            .arg(link)
            .arg(target)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "failed to create a directory junction");
    }
}
