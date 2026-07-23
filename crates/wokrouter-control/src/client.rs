use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    CONTROL_PROTOCOL_VERSION, ControlEndpoint, ControlError, ControlRequest, ControlResponse,
    codec::{read_frame, write_frame},
    protocol::Frame,
    transport::{ClientStream, connect},
};

pub struct ControlClient {
    endpoint: ControlEndpoint,
    protocol_version: u16,
    stream: Mutex<Option<ClientStream>>,
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
            endpoint: endpoint.clone(),
            protocol_version,
            stream: Mutex::new(Some(stream)),
        })
    }

    pub async fn request(&self, request: ControlRequest) -> Result<ControlResponse, ControlError> {
        let mut stream_slot = self.stream.lock().await;
        if stream_slot.is_none() {
            *stream_slot = Some(connect(&self.endpoint).await?);
        }
        let mut stream = stream_slot
            .take()
            .expect("connected control stream must be present");
        let response = async {
            let request_id = Uuid::new_v4();
            let request = Frame {
                protocol_version: self.protocol_version,
                request_id,
                payload: request,
            };
            write_frame(&mut stream, &request).await?;
            let response: Frame<ControlResponse> = read_frame(&mut stream).await?;

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
        .await;
        if response.is_ok() {
            *stream_slot = Some(stream);
        }
        response
    }
}
