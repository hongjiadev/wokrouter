use std::{fs, path::Path};

use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

pub const INSTANCE_ID: &str = "01234567-89ab-4cde-8fab-0123456789ab";
pub const INSTALLATION_ID: &str =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

pub fn write_discovery(
    path: &Path,
    base_url: &str,
    instance_id: &str,
    api_major: u32,
    extra: Option<(&str, serde_json::Value)>,
) {
    write_discovery_with_version(path, base_url, instance_id, "0.1.0", api_major, extra);
}

pub fn write_discovery_with_version(
    path: &Path,
    base_url: &str,
    instance_id: &str,
    wokcore_version: &str,
    api_major: u32,
    extra: Option<(&str, serde_json::Value)>,
) {
    let mut document = json!({
        "base_url": base_url,
        "pid": std::process::id(),
        "instance_id": instance_id,
        "wokcore_version": wokcore_version,
        "api_major": api_major
    });
    if let Some((name, value)) = extra {
        document.as_object_mut().unwrap().insert(name.into(), value);
    }
    fs::write(path, serde_json::to_vec(&document).unwrap()).unwrap();
    secure_file(path);
}

#[allow(dead_code)]
pub async fn mount_handshake(server: &MockServer, instance_id: &str) {
    mount_handshake_with_version(server, instance_id, "0.1.0").await;
}

pub async fn mount_handshake_with_version(
    server: &MockServer,
    instance_id: &str,
    wokcore_version: &str,
) {
    let authority = server.uri().trim_start_matches("http://").to_owned();
    Mock::given(method("GET"))
        .and(path("/wokcore/v1/health"))
        .and(header("host", authority.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "ok",
            "instance_id": instance_id,
            "future_health_field": true
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/wokcore/v1/capabilities"))
        .and(header("host", authority.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "wokcore_version": wokcore_version,
            "management_api_major": 1,
            "minimum_management_api_major": 1,
            "maximum_management_api_major": 1,
            "provider_protocols": ["openai_responses", "anthropic_messages"],
            "capabilities": ["discovery.v1", "service.status"],
            "instance_id": instance_id,
            "installation_id": INSTALLATION_ID,
            "future_capability_field": {"enabled": true}
        })))
        .mount(server)
        .await;
}

#[cfg(unix)]
fn secure_file(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[cfg(windows)]
fn secure_file(path: &Path) {
    use std::{
        ffi::c_void,
        fs::File,
        io,
        mem::size_of,
        os::windows::{
            ffi::OsStrExt,
            fs::MetadataExt,
            io::{AsRawHandle, FromRawHandle},
        },
        ptr,
    };

    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, INVALID_HANDLE_VALUE},
        Security::{
            Authorization::{SE_FILE_OBJECT, SetSecurityInfo},
            GetTokenInformation, OWNER_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER, TokenUser,
        },
        Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, READ_CONTROL,
            WRITE_OWNER,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    fn current_user_sid() -> io::Result<Vec<usize>> {
        let mut token: HANDLE = ptr::null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut required = 0;
        unsafe {
            GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut required);
        }
        if required == 0 {
            unsafe {
                CloseHandle(token);
            }
            return Err(io::Error::last_os_error());
        }
        let mut storage = vec![0_usize; (required as usize).div_ceil(size_of::<usize>())];
        let succeeded = unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                storage.as_mut_ptr().cast::<c_void>(),
                required,
                &mut required,
            )
        } != 0;
        unsafe {
            CloseHandle(token);
        }
        if succeeded {
            Ok(storage)
        } else {
            Err(io::Error::last_os_error())
        }
    }

    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            READ_CONTROL | WRITE_OWNER,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        panic!(
            "failed to open discovery fixture: {}",
            io::Error::last_os_error()
        );
    }
    let file = unsafe { File::from_raw_handle(handle) };
    let metadata = file.metadata().unwrap();
    assert!(metadata.is_file());
    assert_eq!(metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT, 0);

    let user = current_user_sid().unwrap();
    let user = unsafe { &*(user.as_ptr().cast::<TOKEN_USER>()) };
    let status = unsafe {
        SetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            user.User.Sid,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    assert_eq!(status, ERROR_SUCCESS);
}

#[cfg(not(any(unix, windows)))]
fn secure_file(_path: &Path) {}
