use std::{
    ffi::OsStr,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::PlatformError;

const MAX_INSTALL_RECORD_BYTES: usize = 8 * 1024;
const INSTALL_RECORD_SCHEMA_VERSION: u32 = 1;

pub fn discover_wokcore_executable(
    install_record: &Path,
) -> Result<Option<PathBuf>, PlatformError> {
    match read_install_record(install_record)? {
        Some(executable) => Ok(Some(executable)),
        None => Ok(search_path()),
    }
}

fn read_install_record(path: &Path) -> Result<Option<PathBuf>, PlatformError> {
    let Some(mut file) = open_secure(path)? else {
        return Ok(None);
    };
    let metadata = file
        .metadata()
        .map_err(|_| PlatformError::InvalidWokCoreInstallRecord)?;
    if !metadata.is_file() || metadata.len() > MAX_INSTALL_RECORD_BYTES as u64 {
        return Err(PlatformError::InvalidWokCoreInstallRecord);
    }
    let mut bytes = Vec::with_capacity((metadata.len() as usize).min(MAX_INSTALL_RECORD_BYTES));
    file.by_ref()
        .take((MAX_INSTALL_RECORD_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| PlatformError::InvalidWokCoreInstallRecord)?;
    if bytes.len() > MAX_INSTALL_RECORD_BYTES {
        return Err(PlatformError::InvalidWokCoreInstallRecord);
    }
    let record = serde_json::from_slice::<InstallRecord>(&bytes)
        .map_err(|_| PlatformError::InvalidWokCoreInstallRecord)?;
    if record.schema_version != INSTALL_RECORD_SCHEMA_VERSION
        || !record.executable.is_absolute()
        || !valid_executable(&record.executable)
    {
        return Err(PlatformError::InvalidWokCoreInstallRecord);
    }
    Ok(Some(record.executable))
}

fn search_path() -> Option<PathBuf> {
    let search_path = std::env::var_os("PATH")?;
    std::env::split_paths(&search_path)
        .map(|directory| directory.join(executable_name()))
        .find(|candidate| valid_executable(candidate))
}

#[cfg(windows)]
fn executable_name() -> &'static OsStr {
    OsStr::new("wokcore.exe")
}

#[cfg(not(windows))]
fn executable_name() -> &'static OsStr {
    OsStr::new("wokcore")
}

fn valid_executable(path: &Path) -> bool {
    if path.file_name() != Some(executable_name()) {
        return false;
    }
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return false;
    }
    valid_platform_executable(&metadata)
}

#[cfg(unix)]
fn valid_platform_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    metadata.uid() == unsafe { libc::geteuid() } && metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn valid_platform_executable(_metadata: &std::fs::Metadata) -> bool {
    true
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstallRecord {
    schema_version: u32,
    executable: PathBuf,
}

#[cfg(unix)]
fn open_secure(path: &Path) -> Result<Option<File>, PlatformError> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(PlatformError::InvalidWokCoreInstallRecord),
    };
    let metadata = file
        .metadata()
        .map_err(|_| PlatformError::InvalidWokCoreInstallRecord)?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(PlatformError::InvalidWokCoreInstallRecord);
    }
    Ok(Some(file))
}

#[cfg(windows)]
fn open_secure(path: &Path) -> Result<Option<File>, PlatformError> {
    use std::{
        os::windows::{ffi::OsStrExt, fs::MetadataExt, io::FromRawHandle},
        ptr,
    };

    use windows_sys::Win32::{
        Foundation::{
            ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, GENERIC_READ, INVALID_HANDLE_VALUE,
        },
        Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, READ_CONTROL,
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
            GENERIC_READ | READ_CONTROL,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        let error = std::io::Error::last_os_error();
        return match error.raw_os_error() {
            Some(code)
                if code == ERROR_FILE_NOT_FOUND as i32 || code == ERROR_PATH_NOT_FOUND as i32 =>
            {
                Ok(None)
            }
            _ => Err(PlatformError::InvalidWokCoreInstallRecord),
        };
    }
    let file = unsafe { File::from_raw_handle(handle) };
    let metadata = file
        .metadata()
        .map_err(|_| PlatformError::InvalidWokCoreInstallRecord)?;
    if !metadata.is_file()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || !owned_by_current_user(&file)
    {
        return Err(PlatformError::InvalidWokCoreInstallRecord);
    }
    Ok(Some(file))
}

#[cfg(windows)]
fn owned_by_current_user(file: &File) -> bool {
    use std::{ffi::c_void, mem::size_of, os::windows::io::AsRawHandle, ptr};

    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, LocalFree},
        Security::{
            Authorization::{GetSecurityInfo, SE_FILE_OBJECT},
            EqualSid, GetTokenInformation, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
            TOKEN_QUERY, TOKEN_USER, TokenUser,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    struct Descriptor(PSECURITY_DESCRIPTOR);
    impl Drop for Descriptor {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    LocalFree(self.0.cast());
                }
            }
        }
    }
    struct Token(HANDLE);
    impl Drop for Token {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    let mut owner: PSID = ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    let _descriptor = Descriptor(descriptor);
    if status != ERROR_SUCCESS || owner.is_null() {
        return false;
    }

    let mut token_handle: HANDLE = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token_handle) } == 0 {
        return false;
    }
    let token = Token(token_handle);
    let mut required = 0;
    unsafe {
        GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut required);
    }
    if required == 0 {
        return false;
    }
    let mut buffer = vec![0_usize; (required as usize).div_ceil(size_of::<usize>())];
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast::<c_void>(),
            required,
            &mut required,
        )
    } == 0
    {
        return false;
    }
    let token_user = unsafe { &*(buffer.as_ptr().cast::<TOKEN_USER>()) };
    unsafe { EqualSid(owner, token_user.User.Sid) != 0 }
}

#[cfg(not(any(unix, windows)))]
fn open_secure(_path: &Path) -> Result<Option<File>, PlatformError> {
    Err(PlatformError::InvalidWokCoreInstallRecord)
}
