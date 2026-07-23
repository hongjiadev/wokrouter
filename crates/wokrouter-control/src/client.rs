use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    CONTROL_PROTOCOL_VERSION, ControlEndpoint, ControlError, ControlRequest, ControlResponse,
    codec::{read_frame, write_frame},
    protocol::Frame,
    transport::{ClientStream, connect},
};

pub struct ControlClient {
    protocol_version: u16,
    stream: Mutex<ClientStream>,
}

impl ControlClient {
    pub async fn connect(endpoint: &ControlEndpoint) -> Result<Self, ControlError> {
        Self::connect_with_protocol_version(endpoint, CONTROL_PROTOCOL_VERSION).await
    }

    pub async fn connect_with_protocol_version(
        endpoint: &ControlEndpoint,
        protocol_version: u16,
    ) -> Result<Self, ControlError> {
        let stream = connect(endpoint).await?;
        Ok(Self {
            protocol_version,
            stream: Mutex::new(stream),
        })
    }

    pub async fn request(&self, request: ControlRequest) -> Result<ControlResponse, ControlError> {
        let request_id = Uuid::new_v4();
        let request = Frame {
            protocol_version: self.protocol_version,
            request_id,
            payload: request,
        };
        let mut stream = self.stream.lock().await;
        write_frame(&mut *stream, &request).await?;
        let response: Frame<ControlResponse> = read_frame(&mut *stream).await?;

        if response.request_id != request_id {
            return Err(ControlError::RequestIdMismatch);
        }
        if let ControlResponse::Error(error) = response.payload {
            return Err(error);
        }
        if response.protocol_version != self.protocol_version {
            return Err(ControlError::IncompatibleVersion {
                client: self.protocol_version,
                daemon: response.protocol_version,
            });
        }
        Ok(response.payload)
    }
}
