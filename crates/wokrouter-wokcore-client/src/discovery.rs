use std::{fs::File, io::Read, num::NonZeroU32, path::Path};

use semver::Version;
use serde::Deserialize;
use url::{Host, Url};
use uuid::Uuid;

pub(crate) const MAX_DISCOVERY_BYTES: usize = 16 * 1024;

pub(crate) enum DiscoveryRead {
    Missing,
    Invalid,
    Record(ValidatedDiscovery),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ValidatedDiscovery {
    pub base_url: Url,
    pub authority: String,
    pub process_id: NonZeroU32,
    pub instance_id: Uuid,
    pub wokcore_version: Version,
    pub api_major: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DiscoveryWire {
    base_url: String,
    pid: u32,
    instance_id: String,
    wokcore_version: String,
    api_major: u32,
}

pub(crate) fn read(path: &Path) -> DiscoveryRead {
    let mut file = match open_secure(path) {
        Ok(Some(file)) => file,
        Ok(None) => return DiscoveryRead::Missing,
        Err(()) => return DiscoveryRead::Invalid,
    };
    let metadata = match file.metadata() {
        Ok(metadata) if metadata.is_file() && metadata.len() <= MAX_DISCOVERY_BYTES as u64 => {
            metadata
        }
        _ => return DiscoveryRead::Invalid,
    };
    let mut bytes = Vec::with_capacity((metadata.len() as usize).min(MAX_DISCOVERY_BYTES));
    if file
        .by_ref()
        .take((MAX_DISCOVERY_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() > MAX_DISCOVERY_BYTES
    {
        return DiscoveryRead::Invalid;
    }
    let wire = match serde_json::from_slice::<DiscoveryWire>(&bytes) {
        Ok(wire) => wire,
        Err(_) => return DiscoveryRead::Invalid,
    };
    match validate(wire) {
        Some(record) => DiscoveryRead::Record(record),
        None => DiscoveryRead::Invalid,
    }
}

fn validate(wire: DiscoveryWire) -> Option<ValidatedDiscovery> {
    if wire.api_major == 0 {
        return None;
    }
    let process_id = NonZeroU32::new(wire.pid)?;
    let base_url = Url::parse(&wire.base_url).ok()?;
    let port = base_url.port()?;
    let valid_url = base_url.scheme() == "http"
        && base_url.host() == Some(Host::Ipv4(std::net::Ipv4Addr::LOCALHOST))
        && base_url.username().is_empty()
        && base_url.password().is_none()
        && base_url.path() == "/"
        && base_url.query().is_none()
        && base_url.fragment().is_none()
        && port != 0;
    if !valid_url {
        return None;
    }

    Some(ValidatedDiscovery {
        base_url,
        authority: format!("127.0.0.1:{port}"),
        process_id,
        instance_id: Uuid::parse_str(&wire.instance_id).ok()?,
        wokcore_version: Version::parse(&wire.wokcore_version).ok()?,
        api_major: wire.api_major,
    })
}

#[cfg(unix)]
fn open_secure(path: &Path) -> Result<Option<File>, ()> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(()),
    };
    let metadata = file.metadata().map_err(|_| ())?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(());
    }
    Ok(Some(file))
}

#[cfg(windows)]
fn open_secure(path: &Path) -> Result<Option<File>, ()> {
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
            _ => Err(()),
        };
    }
    let file = unsafe { File::from_raw_handle(handle) };
    let metadata = file.metadata().map_err(|_| ())?;
    if !metadata.is_file()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || !owned_by_current_user(&file)
    {
        return Err(());
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
fn open_secure(_path: &Path) -> Result<Option<File>, ()> {
    Err(())
}
