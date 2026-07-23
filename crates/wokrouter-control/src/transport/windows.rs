use std::{ffi::c_void, io, mem::size_of, ptr, time::Duration};

use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND,
        ERROR_PIPE_BUSY, HANDLE, LocalFree,
    },
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SDDL_REVISION_1,
        },
        GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
        TokenUser,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

use super::ControlEndpoint;
use crate::ControlError;

pub(crate) type ClientStream = NamedPipeClient;
pub(crate) type ServerStream = NamedPipeServer;

pub(crate) struct Listener {
    endpoint: ControlEndpoint,
    pending: Option<NamedPipeServer>,
}

impl Listener {
    pub(crate) async fn accept(&mut self) -> io::Result<ServerStream> {
        self.pending
            .as_ref()
            .ok_or_else(|| io::Error::other("named pipe listener is not initialized"))?
            .connect()
            .await?;
        let server = self.pending.take().expect("connected pipe must be pending");
        self.pending = Some(create_server(&self.endpoint, false)?);
        Ok(server)
    }
}

pub(crate) async fn bind(endpoint: &ControlEndpoint) -> Result<Listener, ControlError> {
    let pending = create_server(endpoint, true).map_err(|error| match error.raw_os_error() {
        Some(code) if code == ERROR_ACCESS_DENIED as i32 || code == ERROR_PIPE_BUSY as i32 => {
            ControlError::EndpointInUse
        }
        _ => error.into(),
    })?;
    Ok(Listener {
        endpoint: endpoint.clone(),
        pending: Some(pending),
    })
}

pub(crate) async fn connect(endpoint: &ControlEndpoint) -> Result<ClientStream, ControlError> {
    for _ in 0..100 {
        match ClientOptions::new().open(endpoint.as_pipe_name()) {
            Ok(stream) => return Ok(stream),
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(error)
                if matches!(
                    error.raw_os_error(),
                    Some(code)
                        if code == ERROR_FILE_NOT_FOUND as i32
                            || code == ERROR_PATH_NOT_FOUND as i32
                ) =>
            {
                return Err(ControlError::EndpointUnavailable);
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err(io::Error::from_raw_os_error(ERROR_PIPE_BUSY as i32).into())
}

fn create_server(endpoint: &ControlEndpoint, first: bool) -> io::Result<NamedPipeServer> {
    let descriptor = CurrentUserSecurityDescriptor::new()?;
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let mut options = ServerOptions::new();
    options
        .first_pipe_instance(first)
        .reject_remote_clients(true);
    unsafe {
        options.create_with_security_attributes_raw(
            endpoint.as_pipe_name(),
            (&mut attributes as *mut SECURITY_ATTRIBUTES).cast::<c_void>(),
        )
    }
}

struct CurrentUserSecurityDescriptor(PSECURITY_DESCRIPTOR);

impl CurrentUserSecurityDescriptor {
    fn new() -> io::Result<Self> {
        let mut token_handle: HANDLE = ptr::null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token_handle) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = OwnedHandle(token_handle);
        let mut required = 0;
        unsafe { GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut required) };
        if required == 0 {
            return Err(io::Error::last_os_error());
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
            return Err(io::Error::last_os_error());
        }
        let token_user = unsafe { &*(buffer.as_ptr().cast::<TOKEN_USER>()) };

        let mut sid_string = ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut sid_string) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let sid_string = LocalWideString(sid_string);
        let sid = unsafe { wide_string(sid_string.0) };
        let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid})")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor = ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(descriptor))
    }
}

impl Drop for CurrentUserSecurityDescriptor {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { LocalFree(self.0.cast()) };
        }
    }
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

struct LocalWideString(*mut u16);

impl Drop for LocalWideString {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { LocalFree(self.0.cast()) };
        }
    }
}

unsafe fn wide_string(pointer: *const u16) -> String {
    let mut length = 0;
    while unsafe { *pointer.add(length) } != 0 {
        length += 1;
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(pointer, length) })
}
