use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

#[cfg(windows)]
use super::windows_security::{
    PrivatePathKind, private_path_owned_by_current_user_and_system, secure_private_path,
};

const DIRECTORY_ANCHOR: &str = ".private-path-anchor";

pub struct PrivateDirectoryGuard {
    path: PathBuf,
    private_boundary: PathBuf,
    anchor: PathBuf,
    directories: Vec<(PathBuf, File)>,
}

pub fn secure_private_file(path: &Path) -> io::Result<()> {
    secure(path, PrivateKind::File)
}

pub fn secure_private_directory(path: &Path) -> io::Result<()> {
    secure(path, PrivateKind::Directory)
}

pub fn is_private_file(path: &Path) -> bool {
    is_private(path, PrivateKind::File)
}

pub fn is_private_directory(path: &Path) -> bool {
    is_private(path, PrivateKind::Directory)
}

pub fn pin_private_directory(
    path: &Path,
    private_boundary: &Path,
) -> io::Result<PrivateDirectoryGuard> {
    if !path.is_absolute()
        || !private_boundary.is_absolute()
        || !path.starts_with(private_boundary)
        || !is_private_directory(path)
        || !is_private_directory(private_boundary)
    {
        return Err(unsafe_path());
    }
    let anchor = path.join(DIRECTORY_ANCHOR);
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    configure_private_create(&mut options);
    let anchor_file = options.open(&anchor)?;
    secure_private_file(&anchor)?;
    if !is_private_file(&anchor) {
        return Err(unsafe_path());
    }
    drop(anchor_file);
    if !ancestor_chain_is_safe(&anchor) {
        return Err(unsafe_path());
    }

    let mut paths = path.ancestors().map(Path::to_path_buf).collect::<Vec<_>>();
    paths.reverse();
    let mut directories = Vec::with_capacity(paths.len());
    for directory in paths {
        directories.push((directory.clone(), pin_directory(&directory)?));
    }

    let guard = PrivateDirectoryGuard {
        path: path.to_path_buf(),
        private_boundary: private_boundary.to_path_buf(),
        anchor,
        directories,
    };
    guard.verify().then_some(guard).ok_or_else(unsafe_path)
}

impl PrivateDirectoryGuard {
    pub fn verify(&self) -> bool {
        is_private_directory(&self.path)
            && is_private_directory(&self.private_boundary)
            && is_private_file(&self.anchor)
            && ancestor_chain_is_safe(&self.anchor)
            && self
                .directories
                .iter()
                .all(|(path, file)| directory_handle_matches_path(path, file))
    }
}

#[derive(Clone, Copy)]
enum PrivateKind {
    File,
    Directory,
}

fn unsafe_path() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "unsafe private path")
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
fn ancestor_chain_is_safe(_anchor: &Path) -> bool {
    true
}

#[cfg(unix)]
fn ancestor_chain_is_safe(anchor: &Path) -> bool {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let current_user = unsafe { libc::geteuid() };
    anchor.ancestors().enumerate().all(|(index, path)| {
        let Ok(metadata) = fs::symlink_metadata(path) else {
            return false;
        };
        let expected_kind = if index == 0 {
            metadata.is_file()
        } else {
            metadata.is_dir()
        };
        let mode = metadata.permissions().mode();
        let owner_is_trusted = metadata.uid() == current_user || metadata.uid() == 0;
        let untrusted_write = mode & 0o022 != 0;
        let sticky_directory = index != 0 && mode & libc::S_ISVTX as u32 != 0;
        expected_kind
            && !metadata.file_type().is_symlink()
            && owner_is_trusted
            && (!untrusted_write || sticky_directory)
    })
}

#[cfg(not(any(unix, windows)))]
fn ancestor_chain_is_safe(_anchor: &Path) -> bool {
    false
}

#[cfg(windows)]
fn pin_directory(path: &Path) -> io::Result<File> {
    use std::{os::windows::ffi::OsStrExt, os::windows::io::FromRawHandle, ptr};

    use windows_sys::Win32::{
        Foundation::INVALID_HANDLE_VALUE,
        Storage::FileSystem::{
            CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
            FILE_SHARE_WRITE, OPEN_EXISTING, READ_CONTROL,
        },
    };

    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            READ_CONTROL,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_handle(handle) })
}

#[cfg(unix)]
fn pin_directory(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(any(unix, windows)))]
fn pin_directory(_path: &Path) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "private runtime paths are unsupported on this platform",
    ))
}

#[cfg(windows)]
fn directory_handle_matches_path(path: &Path, file: &File) -> bool {
    use std::{os::windows::fs::MetadataExt, os::windows::io::AsRawHandle};

    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, GetFileInformationByHandle,
    };

    let Ok(handle_metadata) = file.metadata() else {
        return false;
    };
    let Ok(path_metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    let Ok(path_file) = pin_directory(path) else {
        return false;
    };
    let identity = |file: &File| {
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        (unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } != 0)
            .then_some((
                information.dwVolumeSerialNumber,
                ((information.nFileIndexHigh as u64) << 32) | information.nFileIndexLow as u64,
            ))
    };
    handle_metadata.is_dir()
        && handle_metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
        && path_metadata.is_dir()
        && path_metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
        && identity(file).is_some()
        && identity(file) == identity(&path_file)
}

#[cfg(unix)]
fn directory_handle_matches_path(path: &Path, file: &File) -> bool {
    use std::os::unix::fs::MetadataExt;

    let Ok(handle_metadata) = file.metadata() else {
        return false;
    };
    fs::symlink_metadata(path).is_ok_and(|path_metadata| {
        path_metadata.is_dir()
            && !path_metadata.file_type().is_symlink()
            && handle_metadata.dev() == path_metadata.dev()
            && handle_metadata.ino() == path_metadata.ino()
    })
}

#[cfg(not(any(unix, windows)))]
fn directory_handle_matches_path(_path: &Path, _file: &File) -> bool {
    false
}

#[cfg(unix)]
fn secure(path: &Path, kind: PrivateKind) -> io::Result<()> {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let mode = match kind {
        PrivateKind::File => 0o600,
        PrivateKind::Directory => 0o700,
    };
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(windows)]
fn secure(path: &Path, kind: PrivateKind) -> io::Result<()> {
    let kind = match kind {
        PrivateKind::File => PrivatePathKind::File,
        PrivateKind::Directory => PrivatePathKind::Directory,
    };
    secure_private_path(path, kind)
}

#[cfg(unix)]
fn is_private(path: &Path, kind: PrivateKind) -> bool {
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    fs::symlink_metadata(path).is_ok_and(|metadata| {
        let expected_kind = match kind {
            PrivateKind::File => metadata.is_file(),
            PrivateKind::Directory => metadata.is_dir(),
        };
        expected_kind
            && !metadata.file_type().is_symlink()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.permissions().mode() & 0o077 == 0
    })
}

#[cfg(windows)]
fn is_private(path: &Path, kind: PrivateKind) -> bool {
    let kind = match kind {
        PrivateKind::File => PrivatePathKind::File,
        PrivateKind::Directory => PrivatePathKind::Directory,
    };
    private_path_owned_by_current_user_and_system(path, kind)
}

#[cfg(not(any(unix, windows)))]
fn secure(_path: &Path, _kind: PrivateKind) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "private runtime paths are unsupported on this platform",
    ))
}

#[cfg(not(any(unix, windows)))]
fn is_private(_path: &Path, _kind: PrivateKind) -> bool {
    false
}

#[cfg(all(test, windows))]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{pin_private_directory, secure_private_directory};

    #[test]
    fn a_private_directory_under_the_user_temp_root_can_be_pinned() {
        let fixture = tempdir().unwrap();
        let runtime = fixture.path().join("runtime");
        fs::create_dir(&runtime).unwrap();
        secure_private_directory(fixture.path()).unwrap();
        secure_private_directory(&runtime).unwrap();

        let guard = pin_private_directory(&runtime, fixture.path()).unwrap();

        assert!(guard.verify());
    }
}
