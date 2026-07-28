use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::Path,
};

use tempfile::NamedTempFile;

use super::journal::MutationError;

pub(super) fn replace_private_file(path: &Path, contents: &[u8]) -> Result<(), MutationError> {
    let parent = path.parent().ok_or(MutationError::UnsafeTarget)?;
    let mut temporary = NamedTempFile::new_in(parent).map_err(|_| MutationError::Io)?;
    temporary
        .write_all(contents)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|_| MutationError::Io)?;
    secure_file(temporary.path())?;
    let temporary = temporary.into_temp_path();
    replace_file(temporary.as_ref(), path).map_err(|_| MutationError::Io)?;
    secure_file(path)?;
    sync_directory(parent)?;
    Ok(())
}

pub(super) fn write_new_private_file(path: &Path, contents: &[u8]) -> Result<(), MutationError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| MutationError::Io)?;
    file.write_all(contents)
        .and_then(|()| file.sync_all())
        .map_err(|_| MutationError::Io)?;
    secure_file(path)?;
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

pub(super) fn secure_existing_file(path: &Path) -> Result<(), MutationError> {
    secure_file(path)
}

#[cfg(unix)]
pub(super) fn private_file(path: &Path) -> bool {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.permissions().mode() & 0o077 == 0
    })
}

#[cfg(windows)]
pub(super) fn private_file(path: &Path) -> bool {
    crate::system::windows_security::private_path_owned_by_current_user_and_system(
        path,
        crate::system::windows_security::PrivatePathKind::File,
    )
}

#[cfg(not(any(unix, windows)))]
pub(super) fn private_file(_path: &Path) -> bool {
    false
}

pub(super) fn create_private_directory(path: &Path) -> Result<(), MutationError> {
    fs::create_dir_all(path).map_err(|_| MutationError::Io)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| MutationError::Io)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(MutationError::InvalidRecord);
    }
    secure_directory(path)
}

pub(super) fn remove_private_file(path: &Path) -> Result<(), MutationError> {
    match fs::remove_file(path) {
        Ok(()) => {
            if let Some(parent) = path.parent() {
                sync_directory(parent)?;
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(MutationError::Io),
    }
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    if !destination.exists() {
        return fs::rename(source, destination);
    }
    let source = wide_path(source);
    let destination = wide_path(destination);
    let replaced = unsafe {
        windows_sys::Win32::Storage::FileSystem::ReplaceFileW(
            destination.as_ptr(),
            source.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if replaced == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(windows)]
fn wide_path(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;

    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), MutationError> {
    std::fs::File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|_| MutationError::Io)
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), MutationError> {
    Ok(())
}

#[cfg(unix)]
fn secure_file(path: &Path) -> Result<(), MutationError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|_| MutationError::Io)
}

#[cfg(unix)]
fn secure_directory(path: &Path) -> Result<(), MutationError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|_| MutationError::Io)
}

#[cfg(windows)]
fn secure_file(path: &Path) -> Result<(), MutationError> {
    crate::system::windows_security::secure_private_path(
        path,
        crate::system::windows_security::PrivatePathKind::File,
    )
    .map_err(|_| MutationError::Io)
}

#[cfg(windows)]
fn secure_directory(path: &Path) -> Result<(), MutationError> {
    crate::system::windows_security::secure_private_path(
        path,
        crate::system::windows_security::PrivatePathKind::Directory,
    )
    .map_err(|_| MutationError::Io)
}

#[cfg(not(any(unix, windows)))]
fn secure_file(_path: &Path) -> Result<(), MutationError> {
    Err(MutationError::UnsupportedPlatform)
}

#[cfg(not(any(unix, windows)))]
fn secure_directory(_path: &Path) -> Result<(), MutationError> {
    Err(MutationError::UnsupportedPlatform)
}
