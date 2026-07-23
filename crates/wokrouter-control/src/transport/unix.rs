use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io,
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use fs4::fs_std::FileExt;
use tokio::net::{UnixListener, UnixStream};

use super::{ControlEndpoint, EndpointIdentity, stale_socket_removal_allowed};
use crate::ControlError;

pub(crate) type ClientStream = UnixStream;
pub(crate) type ServerStream = UnixStream;

pub(crate) struct Listener {
    inner: UnixListener,
    endpoint: PathBuf,
    identity: EndpointIdentity,
    _bind_lock: File,
}

impl Listener {
    pub(crate) async fn accept(&mut self) -> io::Result<ServerStream> {
        self.inner.accept().await.map(|(stream, _)| stream)
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        remove_if_same_socket(&self.endpoint, self.identity);
    }
}

pub(crate) async fn connect(endpoint: &ControlEndpoint) -> Result<ClientStream, ControlError> {
    UnixStream::connect(endpoint.as_path())
        .await
        .map_err(Into::into)
}

pub(crate) async fn bind(endpoint: &ControlEndpoint) -> Result<Listener, ControlError> {
    let bind_lock = acquire_bind_lock(endpoint)?;
    match UnixListener::bind(endpoint.as_path()) {
        Ok(listener) => secure(listener, endpoint, bind_lock),
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
            let probed =
                socket_identity(endpoint.as_path()).map_err(|_| ControlError::EndpointInUse)?;
            if !probed.is_socket {
                return Err(ControlError::EndpointInUse);
            }
            match UnixStream::connect(endpoint.as_path()).await {
                Ok(_) => Err(ControlError::EndpointInUse),
                Err(connect_error) if connect_error.kind() == io::ErrorKind::ConnectionRefused => {
                    let current = socket_identity(endpoint.as_path()).ok();
                    if !stale_socket_removal_allowed(probed, current) {
                        return Err(ControlError::EndpointInUse);
                    }
                    fs::remove_file(endpoint.as_path())?;
                    secure(UnixListener::bind(endpoint.as_path())?, endpoint, bind_lock)
                }
                Err(connect_error) => Err(connect_error.into()),
            }
        }
        Err(error) => Err(error.into()),
    }
}

fn acquire_bind_lock(endpoint: &ControlEndpoint) -> Result<File, ControlError> {
    let lock_path = bind_lock_path(endpoint.as_path());
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(lock_path)?;
    if !FileExt::try_lock_exclusive(&lock)? {
        return Err(ControlError::EndpointInUse);
    }
    lock.set_permissions(fs::Permissions::from_mode(0o600))?;
    Ok(lock)
}

fn bind_lock_path(endpoint: &Path) -> PathBuf {
    let mut name = OsString::from(endpoint.as_os_str());
    name.push(".lock");
    PathBuf::from(name)
}

fn secure(
    listener: UnixListener,
    endpoint: &ControlEndpoint,
    bind_lock: File,
) -> Result<Listener, ControlError> {
    let identity = socket_identity(endpoint.as_path())?;
    if !identity.is_socket {
        return Err(ControlError::EndpointInUse);
    }
    if let Err(error) = fs::set_permissions(endpoint.as_path(), fs::Permissions::from_mode(0o600)) {
        drop(listener);
        remove_if_same_socket(endpoint.as_path(), identity);
        return Err(error.into());
    }
    Ok(Listener {
        inner: listener,
        endpoint: endpoint.as_path().to_owned(),
        identity,
        _bind_lock: bind_lock,
    })
}

fn socket_identity(path: &Path) -> Result<EndpointIdentity, ControlError> {
    let metadata = fs::symlink_metadata(path)?;
    Ok(EndpointIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        is_socket: metadata.file_type().is_socket(),
    })
}

fn remove_if_same_socket(path: &Path, expected: EndpointIdentity) {
    let current = socket_identity(path).ok();
    if stale_socket_removal_allowed(expected, current) {
        let _ = fs::remove_file(path);
    }
}
