use std::{fs, io, os::unix::fs::PermissionsExt};

use tokio::net::{UnixListener, UnixStream};

use super::ControlEndpoint;
use crate::ControlError;

pub(crate) type ClientStream = UnixStream;
pub(crate) type ServerStream = UnixStream;

pub(crate) struct Listener(UnixListener);

impl Listener {
    pub(crate) async fn accept(&mut self) -> io::Result<ServerStream> {
        self.0.accept().await.map(|(stream, _)| stream)
    }
}

pub(crate) async fn connect(endpoint: &ControlEndpoint) -> Result<ClientStream, ControlError> {
    UnixStream::connect(endpoint.as_path())
        .await
        .map_err(Into::into)
}

pub(crate) async fn bind(endpoint: &ControlEndpoint) -> Result<Listener, ControlError> {
    match UnixListener::bind(endpoint.as_path()) {
        Ok(listener) => secure(listener, endpoint),
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
            match UnixStream::connect(endpoint.as_path()).await {
                Ok(_) => Err(ControlError::EndpointInUse),
                Err(connect_error) if connect_error.kind() == io::ErrorKind::ConnectionRefused => {
                    fs::remove_file(endpoint.as_path())?;
                    secure(UnixListener::bind(endpoint.as_path())?, endpoint)
                }
                Err(connect_error) => Err(connect_error.into()),
            }
        }
        Err(error) => Err(error.into()),
    }
}

fn secure(listener: UnixListener, endpoint: &ControlEndpoint) -> Result<Listener, ControlError> {
    if let Err(error) = fs::set_permissions(endpoint.as_path(), fs::Permissions::from_mode(0o600)) {
        drop(listener);
        let _ = fs::remove_file(endpoint.as_path());
        return Err(error.into());
    }
    Ok(Listener(listener))
}
